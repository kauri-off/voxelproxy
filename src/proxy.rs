use std::{future::Future, net::SocketAddr, time::Duration};

use anyhow::{Context, anyhow};
use mc_protocol::packet::{RawPacket, UncompressedPacket};
use serde_json::json;
use tokio::{
    io::{AsyncRead, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::mpsc::{self, Sender},
    task::JoinSet,
    time::timeout,
};

use crate::{
    events::EventSink,
    packets::universal::status::s2c::StatusResponse,
    resolver::{ServerTarget, resolve_host_port},
};

use crate::{
    controller::{ClientId, Controller, PeerLink, run_client, run_server},
    packets::universal::{
        Intent,
        handshaking::c2s::Handshake,
        login::{
            c2s::{CookieResponse, LoginPluginResponse},
            s2c::{
                CookieRequest, EncryptionRequest, LoginDisconnect, LoginPluginRequest,
                LoginSuccess, SetCompression,
            },
        },
    },
    protocols::{Version, VersionProtocol},
};

pub const DEFAULT_PORT: u16 = 25565;
pub const BIND_PORT: u16 = 25565;
pub const HANDSHAKE_CHANNEL_CAPACITY: usize = 32;
const IO_CHANNEL_CAPACITY: usize = 100;
/// Limit for a client to deliver its Handshake and LoginStart.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// Limit for a whole status ping exchange, including the upstream round trip.
const STATUS_TIMEOUT: Duration = Duration::from_secs(15);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Limit for each upstream packet during the login phase.
const LOGIN_TIMEOUT: Duration = Duration::from_secs(30);
const ACCEPT_RETRY_DELAY: Duration = Duration::from_millis(100);

/// A client that sent a login-intent Handshake followed by its LoginStart.
pub struct LoginClient {
    pub stream: TcpStream,
    pub protocol_version: i32,
    pub server_host: String,
    pub server_port: u16,
    pub login_start: RawPacket,
}

/// Disables Nagle: the proxy relays many small packets and must not delay them.
pub fn tune(stream: &TcpStream) {
    let _ = stream.set_nodelay(true);
}

pub async fn connect(addr: SocketAddr) -> anyhow::Result<TcpStream> {
    let stream = timeout(CONNECT_TIMEOUT, TcpStream::connect(addr))
        .await
        .context("connect timed out")??;
    tune(&stream);
    Ok(stream)
}

pub async fn read_with_timeout<R: AsyncRead + Unpin>(
    stream: &mut R,
    limit: Duration,
) -> anyhow::Result<RawPacket> {
    Ok(timeout(limit, RawPacket::read_async(stream))
        .await
        .context("read timed out")??)
}

/// Accepts connections forever; transient accept errors (fd exhaustion, aborted
/// handshakes) are retried instead of silently ending the listener.
/// Connection tasks are owned by the loop and get aborted when it is dropped.
async fn accept_loop<F, Fut>(listener: TcpListener, mut handle: F)
where
    F: FnMut(TcpStream) -> Fut,
    Fut: Future<Output = anyhow::Result<()>> + Send + 'static,
{
    let mut connections = JoinSet::new();
    loop {
        while connections.try_join_next().is_some() {}
        match listener.accept().await {
            Ok((stream, _addr)) => {
                tune(&stream);
                connections.spawn(handle(stream));
            }
            Err(_) => tokio::time::sleep(ACCEPT_RETRY_DELAY).await,
        }
    }
}

/// Accepts connections from `listener`, reads the Minecraft Handshake packet,
/// proxies status pings to `target`, and sends login-intent clients to `tx`.
pub async fn listen_and_dispatch(
    listener: TcpListener,
    tx: Sender<LoginClient>,
    target: ServerTarget,
) {
    accept_loop(listener, |stream| {
        handle_connection(stream, tx.clone(), Some(target.clone()))
    })
    .await
}

/// Auto-mode variant of `listen_and_dispatch`: status pings are proxied to the
/// server named in each Handshake.
pub async fn listen_and_dispatch_auto(listener: TcpListener, tx: Sender<LoginClient>) {
    accept_loop(listener, |stream| {
        handle_connection(stream, tx.clone(), None)
    })
    .await
}

async fn handle_connection(
    mut stream: TcpStream,
    tx: Sender<LoginClient>,
    status_target: Option<ServerTarget>,
) -> anyhow::Result<()> {
    let handshake: Handshake = read_with_timeout(&mut stream, HANDSHAKE_TIMEOUT)
        .await?
        .as_uncompressed()?
        .deserialize_payload()?;

    match Intent::try_from(handshake.intent.0) {
        Ok(Intent::Status) => {
            // Auto mode keeps the client's own handshake: it was already resolved by vanilla.
            let target = match status_target {
                Some(target) => Some(target),
                None => resolve_host_port(&handshake.server_address, handshake.server_port)
                    .await
                    .map(|t| ServerTarget {
                        host: handshake.server_address.clone(),
                        port: handshake.server_port,
                        addr: t.addr,
                    }),
            };
            if let Some(target) = target {
                timeout(STATUS_TIMEOUT, process_status(stream, target, handshake))
                    .await
                    .context("status timed out")??;
            }
        }
        Ok(Intent::Login) => {
            // Read here rather than in the session loop, so a client that stalls
            // after its Handshake cannot block pairing of everyone else.
            let login_start = read_with_timeout(&mut stream, HANDSHAKE_TIMEOUT).await?;
            tx.send(LoginClient {
                protocol_version: handshake.protocol_version.0,
                server_host: handshake.server_address,
                server_port: handshake.server_port,
                login_start,
                stream,
            })
            .await
            .map_err(|_| anyhow!("dispatcher closed"))?;
        }
        Err(_) => {}
    }
    Ok(())
}

async fn process_status(
    mut stream: TcpStream,
    target: ServerTarget,
    mut handshake: Handshake,
) -> anyhow::Result<()> {
    if target.addr == SocketAddr::from(([127, 0, 0, 1], 25565)) {
        // Status request
        RawPacket::read_async(&mut stream).await?;

        // Status response
        UncompressedPacket::from_packet(&StatusResponse {
            response: json!({
                "version": {
                    "name": "VoxelProxy",
                    "protocol": handshake.protocol_version.0
                },
                "players": {
                    "max": 20,
                    "online": 0,
                    "sample": []
                },
                "description": {
                    "text": "VoxelProxy Работает!"
                }
            })
            .to_string(),
        })?
        .write_async(&mut stream)
        .await?;

        // PING
        RawPacket::read_async(&mut stream)
            .await?
            .write_async(&mut stream)
            .await?;
        return Ok(());
    }
    handshake.server_address = target.host;
    handshake.server_port = target.port;
    let mut remote_stream = connect(target.addr).await?;

    UncompressedPacket::from_packet(&handshake)?
        .write_async(&mut remote_stream)
        .await?;

    // STATUS
    RawPacket::read_async(&mut stream)
        .await?
        .write_async(&mut remote_stream)
        .await?;
    RawPacket::read_async(&mut remote_stream)
        .await?
        .write_async(&mut stream)
        .await?;

    // PING
    RawPacket::read_async(&mut stream)
        .await?
        .write_async(&mut remote_stream)
        .await?;
    RawPacket::read_async(&mut remote_stream)
        .await?
        .write_async(&mut stream)
        .await?;

    Ok(())
}

/// Finishes login and runs the Controller; expects `Handshake` + `LoginStart` already exchanged.
pub async fn run_proxy_session(
    mut primary: TcpStream,
    mut secondary: TcpStream,
    mut remote: TcpStream,
    mut version: Version,
    events: EventSink,
) -> anyhow::Result<()> {
    let mut threshold = None;

    loop {
        let raw = read_with_timeout(&mut remote, LOGIN_TIMEOUT).await?;
        let packet = raw.uncompress(threshold)?;

        match packet.packet_id {
            LoginDisconnect::PACKET_ID => {
                let _ = raw.write_async(&mut primary).await;
                let _ = raw.write_async(&mut secondary).await;
                return Err(anyhow!("Disconnected by server"));
            }
            EncryptionRequest::PACKET_ID => {
                send_login_error(
                    &mut primary,
                    &mut secondary,
                    "Лицензионный сервер не поддерживается\nИспользуйте ViaProxy".to_string(),
                    threshold,
                )
                .await;
                return Err(anyhow!("Licensed server"));
            }
            LoginSuccess::PACKET_ID => {
                raw.write_async(&mut primary).await?;
                raw.write_async(&mut secondary).await?;
                break;
            }
            SetCompression::PACKET_ID => {
                let compression: SetCompression = packet.deserialize_payload()?;
                threshold = Some(compression.threshold.0);

                raw.write_async(&mut primary).await?;
                raw.write_async(&mut secondary).await?;
            }
            // Answered the way a vanilla client without mods/cookies would.
            LoginPluginRequest::PACKET_ID => {
                let request: LoginPluginRequest = packet.deserialize_payload()?;
                UncompressedPacket::from_packet(&LoginPluginResponse {
                    message_id: request.message_id,
                    successful: false,
                })?
                .to_raw_packet_compressed(threshold)?
                .write_async(&mut remote)
                .await?;
            }
            CookieRequest::PACKET_ID => {
                let request: CookieRequest = packet.deserialize_payload()?;
                UncompressedPacket::from_packet(&CookieResponse {
                    key: request.key,
                    payload: None,
                })?
                .to_raw_packet_compressed(threshold)?
                .write_async(&mut remote)
                .await?;
            }
            id => {
                send_login_error(
                    &mut primary,
                    &mut secondary,
                    format!("Неожиданный пакет от сервера при входе: {id:#04x}"),
                    threshold,
                )
                .await;
                return Err(anyhow!("Unexpected login packet {id:#04x}"));
            }
        }
    }

    let (primary_read, primary_write) = primary.into_split();
    let (secondary_read, secondary_write) = secondary.into_split();
    let (remote_read, remote_write) = remote.into_split();

    let (event_tx, event_rx) = mpsc::channel(IO_CHANNEL_CAPACITY);
    let (primary_link, primary_rx) = PeerLink::new();
    let (secondary_link, secondary_rx) = PeerLink::new();
    let (remote_tx, remote_rx) = mpsc::channel(IO_CHANNEL_CAPACITY);

    version.update_threshold(threshold);
    let controller = Controller::new(
        ClientId::Primary,
        primary_link,
        secondary_link,
        remote_tx,
        event_rx,
        version,
        events,
    );

    // Dropping the set aborts the I/O tasks, closing their sockets. That happens both
    // when the controller stops and when this whole session future is cancelled; it is
    // required for a half-open client whose read task is wedged in `read_async`.
    let mut io_tasks = JoinSet::new();
    io_tasks.spawn(run_client(
        primary_read,
        primary_write,
        ClientId::Primary,
        event_tx.clone(),
        primary_rx,
    ));
    io_tasks.spawn(run_client(
        secondary_read,
        secondary_write,
        ClientId::Secondary,
        event_tx.clone(),
        secondary_rx,
    ));
    io_tasks.spawn(run_server(remote_read, remote_write, event_tx, remote_rx));

    controller.run().await;
    Ok(())
}

/// Sends a `LoginDisconnect` packet to a single client.
pub async fn send_login_disconnect<W: AsyncWriteExt + Unpin>(stream: &mut W, message: String) {
    let Ok(disconnect) = UncompressedPacket::from_packet(&LoginDisconnect {
        reason: json!({ "text": message }).to_string(),
    }) else {
        return;
    };

    let _ = disconnect.write_async(stream).await;
}

/// Sends a `LoginDisconnect` packet to both clients, framed for the current `threshold`.
pub async fn send_login_error<W: AsyncWriteExt + Unpin>(
    primary: &mut W,
    secondary: &mut W,
    message: String,
    threshold: Option<i32>,
) {
    let Ok(disconnect) = UncompressedPacket::from_packet(&LoginDisconnect {
        reason: json!({"text": message}).to_string(),
    })
    .and_then(|p| p.to_raw_packet_compressed(threshold)) else {
        return;
    };

    let _ = disconnect.write_async(primary).await;
    let _ = disconnect.write_async(secondary).await;
}

#[cfg(test)]
mod tests {
    use futures::StreamExt;
    use mc_protocol::varint::VarInt;

    use super::*;
    use crate::events::{AppEvent, WhichClient};

    async fn socket_pair() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (connected, accepted) = tokio::join!(TcpStream::connect(addr), listener.accept());
        (accepted.unwrap().0, connected.unwrap())
    }

    fn frame(id: i32, payload: Vec<u8>, threshold: Option<i32>) -> RawPacket {
        UncompressedPacket::new(id, payload)
            .to_raw_packet_compressed(threshold)
            .unwrap()
    }

    async fn sessions_for(
        protocol: i32,
    ) -> (
        TcpStream,
        TcpStream,
        TcpStream,
        impl Future<Output = anyhow::Result<()>>,
        crate::events::EventSink,
        futures::channel::mpsc::UnboundedReceiver<AppEvent>,
    ) {
        let (primary, primary_peer) = socket_pair().await;
        let (secondary, secondary_peer) = socket_pair().await;
        let (remote, remote_peer) = socket_pair().await;
        let (events, events_rx) = EventSink::channel();
        let session = run_proxy_session(
            primary,
            secondary,
            remote,
            Version::from_protocol(protocol).unwrap(),
            events.clone(),
        );
        (
            primary_peer,
            secondary_peer,
            remote_peer,
            session,
            events,
            events_rx,
        )
    }

    #[tokio::test]
    async fn aborting_accept_loop_cancels_connection_tasks() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (started_tx, mut started_rx) = mpsc::channel(1);
        let (alive_tx, alive_rx) = tokio::sync::oneshot::channel::<()>();
        let mut alive_tx = Some(alive_tx);
        let accept = tokio::spawn(accept_loop(listener, move |stream| {
            let started = started_tx.clone();
            let alive = alive_tx.take();
            async move {
                let _held = (stream, alive);
                started.send(()).await.unwrap();
                std::future::pending::<anyhow::Result<()>>().await
            }
        }));

        let _client = TcpStream::connect(addr).await.unwrap();
        started_rx.recv().await.unwrap();
        accept.abort();

        let dropped = tokio::time::timeout(Duration::from_secs(5), alive_rx).await;
        assert!(
            matches!(dropped, Ok(Err(_))),
            "connection task outlived accept loop"
        );
    }

    async fn sessions() -> (
        TcpStream,
        TcpStream,
        TcpStream,
        impl Future<Output = anyhow::Result<()>>,
        crate::events::EventSink,
        futures::channel::mpsc::UnboundedReceiver<AppEvent>,
    ) {
        sessions_for(774).await
    }

    fn encode<T: mc_protocol::packet::PacketId + mc_protocol::ser::Serialize>(
        packet: &T,
        threshold: Option<i32>,
    ) -> RawPacket {
        UncompressedPacket::from_packet(packet)
            .unwrap()
            .to_raw_packet_compressed(threshold)
            .unwrap()
    }

    #[tokio::test]
    async fn rotation_reaches_secondary_after_late_login_ack() {
        use crate::protocols::v1_21_4::packets::{c2s, s2c};

        let (primary, secondary, mut remote, session, _events, _rx) = sessions_for(769).await;
        let session = tokio::spawn(session);
        let threshold = Some(256);
        let (primary_read, mut primary) = primary.into_split();
        let (secondary_read, mut secondary) = secondary.into_split();
        let mut primary_read = tokio::io::BufReader::new(primary_read);
        let mut secondary_read = tokio::io::BufReader::new(secondary_read);

        frame(SetCompression::PACKET_ID, vec![0x80, 0x02], None)
            .write_async(&mut remote)
            .await
            .unwrap();
        frame(LoginSuccess::PACKET_ID, vec![], threshold)
            .write_async(&mut remote)
            .await
            .unwrap();
        for reader in [&mut primary_read, &mut secondary_read] {
            for _ in 0..2 {
                RawPacket::read_async(reader).await.unwrap();
            }
        }

        encode(&c2s::login::LoginAcknowledged {}, threshold)
            .write_async(&mut primary)
            .await
            .unwrap();
        encode(&c2s::configuration::FinishConfiguration {}, threshold)
            .write_async(&mut primary)
            .await
            .unwrap();
        for _ in 0..2 {
            RawPacket::read_async(&mut remote).await.unwrap();
        }
        encode(&s2c::configuration::FinishConfiguration {}, threshold)
            .write_async(&mut remote)
            .await
            .unwrap();
        RawPacket::read_async(&mut secondary_read).await.unwrap();

        encode(&c2s::login::LoginAcknowledged {}, threshold)
            .write_async(&mut secondary)
            .await
            .unwrap();
        encode(&c2s::configuration::FinishConfiguration {}, threshold)
            .write_async(&mut secondary)
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;

        let rot = c2s::game::Rot {
            yaw: 90.0,
            pitch: 30.0,
            on_ground: true,
        };
        encode(&rot, threshold)
            .write_async(&mut primary)
            .await
            .unwrap();
        let relayed = RawPacket::read_async(&mut remote).await.unwrap();
        assert_eq!(
            relayed.uncompress(threshold).unwrap().packet_id,
            c2s::game::Rot::PACKET_ID
        );

        let mirrored = tokio::time::timeout(
            Duration::from_secs(5),
            RawPacket::read_async(&mut secondary_read),
        )
        .await
        .expect("rotation never reached the secondary")
        .unwrap();
        let position: s2c::game::Position = mirrored
            .uncompress(threshold)
            .unwrap()
            .deserialize_payload()
            .unwrap();
        assert_eq!((position.yaw, position.pitch), (90.0, 30.0));

        drop(remote);
        tokio::time::timeout(Duration::from_secs(5), session)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn login_is_forwarded_verbatim_and_plugin_requests_answered() {
        let (mut primary, mut secondary, mut remote, session, _events, _rx) = sessions().await;
        let session = tokio::spawn(session);
        let threshold = Some(256);

        let set_compression = frame(SetCompression::PACKET_ID, vec![0x80, 0x02], None);
        set_compression.write_async(&mut remote).await.unwrap();

        let mut request = Vec::new();
        VarInt(5).write_sync(&mut request).unwrap();
        mc_protocol::ser::Serialize::serialize("velocity:player_info", &mut request).unwrap();
        frame(LoginPluginRequest::PACKET_ID, request, threshold)
            .write_async(&mut remote)
            .await
            .unwrap();
        let response = RawPacket::read_async(&mut remote)
            .await
            .unwrap()
            .uncompress(threshold)
            .unwrap();
        assert_eq!(response.packet_id, LoginPluginResponse::PACKET_ID);
        assert_eq!(response.payload, vec![5, 0]);

        let success = frame(LoginSuccess::PACKET_ID, vec![1; 300], threshold);
        success.write_async(&mut remote).await.unwrap();
        let chunks: Vec<_> = (0..50)
            .map(|i| frame(0x27, vec![i as u8; 100_000], threshold))
            .collect();
        for chunk in &chunks {
            chunk.write_async(&mut remote).await.unwrap();
        }

        for client in [&mut primary, &mut secondary] {
            let mut reader = tokio::io::BufReader::new(client);
            for expected in [&set_compression, &success].into_iter().chain(&chunks) {
                let got = RawPacket::read_async(&mut reader).await.unwrap();
                assert_eq!(got.data, expected.data);
            }
        }

        drop(remote);
        tokio::time::timeout(Duration::from_secs(5), session)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn stalled_inactive_client_is_dropped_without_blocking_active() {
        let (mut primary, secondary, mut remote, session, _events, mut events_rx) =
            sessions().await;
        let session = tokio::spawn(session);

        frame(LoginSuccess::PACKET_ID, vec![], None)
            .write_async(&mut remote)
            .await
            .unwrap();
        let writer = tokio::spawn(async move {
            for i in 0..48 {
                frame(0x27, vec![i as u8; 2_000_000], None)
                    .write_async(&mut remote)
                    .await
                    .unwrap();
            }
            remote
        });

        let mut reader = tokio::io::BufReader::new(&mut primary);
        RawPacket::read_async(&mut reader).await.unwrap();
        for i in 0..48 {
            let got =
                tokio::time::timeout(Duration::from_secs(10), RawPacket::read_async(&mut reader))
                    .await
                    .expect("active client stalled")
                    .unwrap();
            assert_eq!(got.data.last(), Some(&(i as u8)));
        }

        let kicked = tokio::time::timeout(Duration::from_secs(5), async {
            while let Some(event) = events_rx.next().await {
                if let AppEvent::ClientStatus {
                    which: WhichClient::Secondary,
                    online: false,
                } = event
                {
                    return;
                }
            }
        })
        .await;
        assert!(kicked.is_ok(), "stalled secondary was never dropped");

        drop(secondary);
        drop(writer.await.unwrap());
        tokio::time::timeout(Duration::from_secs(5), session)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }
}
