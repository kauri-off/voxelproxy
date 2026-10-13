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
    events::EventSink, packets::universal::status::s2c::StatusResponse, resolver::resolve_host_port,
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
async fn accept_loop<F, Fut>(listener: TcpListener, mut handle: F)
where
    F: FnMut(TcpStream) -> Fut,
    Fut: Future<Output = anyhow::Result<()>> + Send + 'static,
{
    loop {
        match listener.accept().await {
            Ok((stream, _addr)) => {
                tune(&stream);
                tokio::spawn(handle(stream));
            }
            Err(_) => tokio::time::sleep(ACCEPT_RETRY_DELAY).await,
        }
    }
}

/// Accepts connections from `listener`, reads the Minecraft Handshake packet,
/// proxies status pings to `remote_addr`, and sends login-intent clients to `tx`.
pub async fn listen_and_dispatch(
    listener: TcpListener,
    tx: Sender<LoginClient>,
    remote_dns: String,
    remote_addr: SocketAddr,
) {
    accept_loop(listener, |stream| {
        handle_connection(stream, tx.clone(), Some((remote_dns.clone(), remote_addr)))
    })
    .await
}

/// Auto-mode variant of `listen_and_dispatch`: status pings are proxied to the
/// server named in each Handshake.
pub async fn listen_and_dispatch_auto(listener: TcpListener, tx: Sender<LoginClient>) {
    accept_loop(listener, |stream| handle_connection(stream, tx.clone(), None)).await
}

async fn handle_connection(
    mut stream: TcpStream,
    tx: Sender<LoginClient>,
    status_target: Option<(String, SocketAddr)>,
) -> anyhow::Result<()> {
    let handshake: Handshake = read_with_timeout(&mut stream, HANDSHAKE_TIMEOUT)
        .await?
        .as_uncompressed()?
        .deserialize_payload()?;

    match Intent::try_from(handshake.intent.0) {
        Ok(Intent::Status) => {
            let target = match status_target {
                Some(target) => Some(target),
                None => resolve_host_port(
                    &handshake.server_address,
                    handshake.server_port,
                    "minecraft",
                    "tcp",
                )
                .await
                .map(|addr| (handshake.server_address.clone(), addr)),
            };
            if let Some((remote_dns, remote_addr)) = target {
                timeout(
                    STATUS_TIMEOUT,
                    process_status(stream, remote_addr, remote_dns, handshake),
                )
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
    remote_addr: SocketAddr,
    remote_dns: String,
    mut handshake: Handshake,
) -> anyhow::Result<()> {
    if remote_addr == SocketAddr::from(([127, 0, 0, 1], 25565)) {
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
    handshake.server_address = remote_dns;
    let mut remote_stream = connect(remote_addr).await?;

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

/// Runs the post-handshake login sequence and then the proxy Controller.
///
/// # Preconditions
/// - The Minecraft `Handshake` + `LoginStart` have been **read** from both
///   `primary` and `secondary` streams.
/// - The Minecraft `Handshake` + `LoginStart` have been **sent** to `remote`.
/// - `version` has been constructed from the shared protocol number.
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
