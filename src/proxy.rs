use std::net::SocketAddr;

use anyhow::anyhow;
use mc_protocol::packet::{RawPacket, UncompressedPacket};
use serde_json::json;
use tokio::{
    io::AsyncWriteExt,
    net::{TcpListener, TcpStream},
    sync::mpsc::{self, Sender},
};

use crate::resolver::resolve_host_port;

use crate::{
    controller::{ClientId, Controller, run_client, run_server},
    packets::universal::{
        Intent,
        handshaking::c2s::Handshake,
        login::s2c::{EncryptionRequest, LoginDisconnect, LoginSuccess, SetCompression},
    },
    protocols::{Version, VersionProtocol},
};

pub const DEFAULT_PORT: u16 = 25565;
pub const BIND_PORT: u16 = 25565;
pub const HANDSHAKE_CHANNEL_CAPACITY: usize = 32;
const IO_CHANNEL_CAPACITY: usize = 100;

/// Information extracted from a client's Minecraft Handshake packet.
/// Used in automatic mode to determine the real remote server without manual input.
pub struct AutoClientInfo {
    pub stream: TcpStream,
    pub protocol_version: i32,
    pub server_host: String,
    pub server_port: u16,
}

#[inline]
pub async fn read_uncompressed(stream: &mut TcpStream) -> anyhow::Result<UncompressedPacket> {
    Ok(RawPacket::read_async(stream).await?.as_uncompressed()?)
}

/// Accepts connections from `listener`, reads the Minecraft Handshake packet,
/// proxies status pings directly, and sends `(stream, protocol_version)` for
/// login intents to `tx`.
pub async fn listen_and_dispatch(
    listener: TcpListener,
    tx: Sender<(TcpStream, i32)>,
    remote_dns: String,
    remote_addr: SocketAddr,
) {
    while let Ok((stream, _addr)) = listener.accept().await {
        tokio::spawn(handle_connection(
            stream,
            tx.clone(),
            remote_dns.clone(),
            remote_addr,
        ));
    }
}

/// Auto-mode variant of `listen_and_dispatch`.
/// Reads the Minecraft Handshake from each connection, handles status pings directly
/// (resolving the server from the Handshake), and sends login-intent clients as
/// `AutoClientInfo` so the caller can pair and proxy them dynamically.
pub async fn listen_and_dispatch_auto(listener: TcpListener, tx: Sender<AutoClientInfo>) {
    while let Ok((stream, _addr)) = listener.accept().await {
        tokio::spawn(handle_connection_auto(stream, tx.clone()));
    }
}

async fn handle_connection_auto(
    mut stream: TcpStream,
    tx: Sender<AutoClientInfo>,
) -> anyhow::Result<()> {
    let handshake: Handshake = read_uncompressed(&mut stream)
        .await?
        .deserialize_payload()?;

    match Intent::try_from(handshake.intent.0) {
        Ok(Intent::Status) => {
            // Resolve the server from the handshake and proxy the ping directly
            if let Some(remote_addr) = resolve_host_port(
                &handshake.server_address,
                handshake.server_port,
                "minecraft",
                "tcp",
            )
            .await
            {
                process_status(
                    stream,
                    remote_addr,
                    handshake.server_address.clone(),
                    handshake,
                )
                .await?;
            }
        }
        Ok(Intent::Login) => {
            tx.send(AutoClientInfo {
                protocol_version: handshake.protocol_version.0,
                server_host: handshake.server_address,
                server_port: handshake.server_port,
                stream,
            })
            .await?;
        }
        Err(_) => {}
    }
    Ok(())
}

async fn handle_connection(
    mut stream: TcpStream,
    tx: Sender<(TcpStream, i32)>,
    remote_dns: String,
    remote_addr: SocketAddr,
) -> anyhow::Result<()> {
    let handshake: Handshake = read_uncompressed(&mut stream)
        .await?
        .deserialize_payload()?;

    match Intent::try_from(handshake.intent.0) {
        Ok(Intent::Status) => process_status(stream, remote_addr, remote_dns, handshake).await?,
        Ok(Intent::Login) => {
            tx.send((stream, handshake.protocol_version.0)).await?;
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
    handshake.server_address = remote_dns;
    let mut remote_stream = TcpStream::connect(remote_addr).await?;

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
///   `cheat` and `legit` streams.
/// - The Minecraft `Handshake` + `LoginStart` have been **sent** to `remote`.
/// - `version` has been constructed from the shared protocol number.
pub async fn run_proxy_session(
    mut cheat: TcpStream,
    mut legit: TcpStream,
    mut remote: TcpStream,
    mut version: Version,
) -> anyhow::Result<()> {
    let mut threshold = None;

    loop {
        let packet = RawPacket::read_async(&mut remote)
            .await?
            .uncompress(threshold)?;

        match packet.packet_id {
            LoginDisconnect::PACKET_ID => {
                send_login_error(
                    &mut cheat,
                    &mut legit,
                    packet.deserialize_payload::<LoginDisconnect>()?.reason,
                )
                .await;
                return Err(anyhow!("Disconnected by server"));
            }
            EncryptionRequest::PACKET_ID => {
                send_login_error(
                    &mut cheat,
                    &mut legit,
                    "Лицензионный сервер не поддерживается\nИспользуйте ViaProxy".to_string(),
                )
                .await;
                return Err(anyhow!("Licensed server"));
            }
            LoginSuccess::PACKET_ID => {
                let packet = packet.to_raw_packet_compressed(threshold)?;
                packet.write_async(&mut cheat).await?;
                packet.write_async(&mut legit).await?;
                println!("[+] Login success");
                break;
            }
            SetCompression::PACKET_ID => {
                let compression: SetCompression = packet.deserialize_payload()?;
                threshold = Some(compression.threshold.0);

                let packet = packet.to_raw_packet()?;
                packet.write_async(&mut cheat).await?;
                packet.write_async(&mut legit).await?;
                println!("[+] Compression");
            }
            _ => unreachable!(),
        }
    }

    let (cheat_read, cheat_write) = cheat.into_split();
    let (legit_read, legit_write) = legit.into_split();
    let (remote_read, remote_write) = remote.into_split();

    let (event_tx, event_rx) = mpsc::channel(IO_CHANNEL_CAPACITY);
    let (cheat_tx, cheat_rx) = mpsc::channel(IO_CHANNEL_CAPACITY);
    let (legit_tx, legit_rx) = mpsc::channel(IO_CHANNEL_CAPACITY);
    let (remote_tx, remote_rx) = mpsc::channel(IO_CHANNEL_CAPACITY);

    version.update_threshold(threshold);
    let controller = Controller::new(
        ClientId::Cheat,
        cheat_tx,
        legit_tx,
        remote_tx,
        event_rx,
        version,
    );

    tokio::spawn(run_client(
        cheat_read,
        cheat_write,
        ClientId::Cheat,
        event_tx.clone(),
        cheat_rx,
    ));
    tokio::spawn(run_client(
        legit_read,
        legit_write,
        ClientId::Legit,
        event_tx.clone(),
        legit_rx,
    ));
    tokio::spawn(run_server(remote_read, remote_write, event_tx, remote_rx));

    println!("VoxelProxy запущен!");
    controller.run().await;
    Ok(())
}

/// Sends a `LoginDisconnect` packet to both clients.
pub async fn send_login_error<W: AsyncWriteExt + Unpin>(
    cheat: &mut W,
    legit: &mut W,
    message: String,
) {
    let disconnect = UncompressedPacket::from_packet(&LoginDisconnect {
        reason: json!({"text": message}).to_string(),
    })
    .unwrap();

    disconnect.write_async(cheat).await.unwrap();
    disconnect.write_async(legit).await.unwrap();
}
