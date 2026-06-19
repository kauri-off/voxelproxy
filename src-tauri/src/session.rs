#[cfg(target_os = "windows")]
use std::net::{IpAddr, ToSocketAddrs};
use std::sync::Arc;

use mc_protocol::{
    packet::{RawPacket, UncompressedPacket},
    varint::VarInt,
};
#[cfg(target_os = "windows")]
use tauri::AppHandle;
use tauri_specta::Event;
use tokio::sync::Mutex;
use tokio::{
    net::{
        TcpListener, TcpStream,
        tcp::{OwnedReadHalf, OwnedWriteHalf},
    },
    sync::mpsc,
    task::JoinSet,
};

use crate::{
    config,
    events::{ClientStatusEvent, NickNameEvent, ServerAddrEvent, WhichClient},
    logger::Logger,
    packets::universal::{Intent, handshaking::c2s::Handshake},
    protocols::{Version, VersionProtocol},
    proxy::{AutoClientInfo, BIND_PORT, DEFAULT_PORT, HANDSHAKE_CHANNEL_CAPACITY},
    resolver::resolve_host_port,
};

/// Returns true if host resolves to the local machine (loopback).
#[cfg(target_os = "windows")]
fn is_loopback_host(host: &str) -> bool {
    let h = host
        .split('\0')
        .next()
        .unwrap_or(host)
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']');

    if h.is_empty() {
        return false;
    }

    // Literal IP: decide locally, no resolution.
    if let Ok(ip) = h.parse::<IpAddr>() {
        return ip.is_loopback();
    }

    // Hostname: ask the system resolver (it honours the hosts file), so anything
    // mapped to 127.0.0.1 — kubernetes.docker.internal or any custom entry — is
    // caught without a hardcoded list.
    match (h, 0u16).to_socket_addrs() {
        Ok(addrs) => {
            let addrs: Vec<_> = addrs.collect();
            !addrs.is_empty() && addrs.iter().all(|a| a.ip().is_loopback())
        }
        Err(_) => false,
    }
}

/// Resolves when `stream` is closed by the peer, errors, or (unexpectedly)
/// sends more bytes. Assumes the client's `LoginStart` has already been drained,
/// so a healthy client keeps the socket unreadable until it disconnects; used to
/// notice a client that vanished while we wait for its partner to connect. `peek`
/// consumes nothing, so this is cancellation-safe inside `select!`.
async fn wait_for_disconnect(stream: &mut TcpStream) {
    let mut probe = [0u8; 1];
    let _ = stream.peek(&mut probe).await;
}

/// Emits `online: false` for both clients when dropped, i.e. whenever a session
/// ends — early return (version mismatch, unsupported, resolve/connect failure),
/// an `?` error during login, a login-phase abort inside `run_proxy_session`, or
/// a normal disconnect. The `online: true` events are emitted at pairing time
/// (`run_automatic_mode`) or as clients connect (`run_manual_mode`), so without
/// this guard any exit before the controller loop starts would leave the UI
/// desynced, still showing both clients connected. This matters most in auto
/// mode, where a single failed pairing does not end the surrounding accept loop
/// and so never emits `SessionEndedEvent`. Re-emitting on a normal disconnect is
/// idempotent (the controller already marked them offline).
struct ClientStatusOfflineGuard {
    app: AppHandle,
}

impl Drop for ClientStatusOfflineGuard {
    fn drop(&mut self) {
        for which in [WhichClient::Primary, WhichClient::Secondary] {
            ClientStatusEvent {
                which,
                online: false,
            }
            .emit(&self.app)
            .ok();
        }
    }
}

pub async fn run_manual_mode(server_addr: String, app: AppHandle) -> anyhow::Result<()> {
    let _status_guard = ClientStatusOfflineGuard { app: app.clone() };
    let log = Logger::new(&app);
    let (remote_addr, remote_dns) =
        match resolve_host_port(&server_addr, DEFAULT_PORT, "minecraft", "tcp").await {
            Some(addr) => (addr, server_addr),
            None => anyhow::bail!("Не удалось разрешить адрес: \"{}\"", server_addr),
        };

    let listener = match TcpListener::bind(format!("0.0.0.0:{}", BIND_PORT)).await {
        Ok(t) => t,
        Err(e) => anyhow::bail!("Ошибка при создании сокета: {}", e),
    };

    let (tx, mut rx) = mpsc::channel(HANDSHAKE_CHANNEL_CAPACITY);
    let mut _dispatch_set: JoinSet<()> = JoinSet::new();
    _dispatch_set.spawn(crate::proxy::listen_and_dispatch(
        listener,
        tx,
        remote_dns.clone(),
        remote_addr,
    ));

    // Acquire a live primary, then wait for the secondary while watching the
    // primary for an early disconnect. If the primary drops before the secondary
    // arrives, mark it offline and re-acquire a fresh primary instead of carrying
    // a dead stream into the session.
    let (
        mut primary_stream,
        primary_protocol,
        primary_login_start,
        mut secondary_stream,
        secondary_protocol,
    ) = loop {
        let (mut primary_stream, primary_protocol) = match rx.recv().await {
            Some(p) => p,
            None => anyhow::bail!("Диспетчер подключений завершился"),
        };
        let primary_login_start = RawPacket::read_async(&mut primary_stream).await?;
        ClientStatusEvent {
            which: WhichClient::Primary,
            online: true,
        }
        .emit(&app)
        .ok();

        enum Sec {
            Got((TcpStream, i32)),
            PrimaryGone,
            Closed,
        }
        let sec = tokio::select! {
            recv = rx.recv() => match recv {
                Some(s) => Sec::Got(s),
                None => Sec::Closed,
            },
            _ = wait_for_disconnect(&mut primary_stream) => Sec::PrimaryGone,
        };
        match sec {
            Sec::Got((secondary_stream, secondary_protocol)) => {
                break (
                    primary_stream,
                    primary_protocol,
                    primary_login_start,
                    secondary_stream,
                    secondary_protocol,
                );
            }
            Sec::PrimaryGone => {
                log.warn("Основной клиент отключился, не дождавшись второго");
                ClientStatusEvent {
                    which: WhichClient::Primary,
                    online: false,
                }
                .emit(&app)
                .ok();
                continue;
            }
            Sec::Closed => anyhow::bail!("Диспетчер подключений завершился"),
        }
    };
    let _ = RawPacket::read_async(&mut secondary_stream).await?;
    ClientStatusEvent {
        which: WhichClient::Secondary,
        online: true,
    }
    .emit(&app)
    .ok();

    if primary_protocol != secondary_protocol {
        crate::proxy::send_login_error(
            &mut primary_stream,
            &mut secondary_stream,
            "Версии клиентов различаются".to_string(),
        )
        .await;
        anyhow::bail!("Версии клиентов различаются");
    }

    let version = match Version::from_protocol(primary_protocol) {
        Some(v) => v,
        None => {
            crate::proxy::send_login_error(
                &mut primary_stream,
                &mut secondary_stream,
                "Данная версия не поддерживается".to_string(),
            )
            .await;
            anyhow::bail!("Данная версия не поддерживается");
        }
    };

    let mut remote_stream = match TcpStream::connect(remote_addr).await {
        Ok(t) => t,
        Err(_) => {
            crate::proxy::send_login_error(
                &mut primary_stream,
                &mut secondary_stream,
                "Ошибка при подключении к удалённому серверу".to_string(),
            )
            .await;
            anyhow::bail!("Ошибка при подключении к удалённому серверу");
        }
    };
    log.success(format!("Подключено к {}", remote_addr));

    let handshake = Handshake {
        protocol_version: VarInt(primary_protocol),
        server_address: remote_dns,
        server_port: DEFAULT_PORT,
        intent: Intent::Login.into(),
    };
    UncompressedPacket::from_packet(&handshake)?
        .write_async(&mut remote_stream)
        .await?;

    primary_login_start.write_async(&mut remote_stream).await?;

    let nickname = version
        .parse_login_start(&primary_login_start)
        .unwrap_or("...".to_string());
    NickNameEvent(nickname.clone()).emit(&app).ok();
    ServerAddrEvent(handshake.server_address.clone())
        .emit(&app)
        .ok();
    tokio::spawn(config::send_join(
        handshake.server_address.clone(),
        nickname,
        handshake.protocol_version.0,
    ));

    crate::proxy::run_proxy_session(
        primary_stream,
        secondary_stream,
        remote_stream,
        version,
        app,
    )
    .await
}

async fn run_auto_session(
    mut primary: AutoClientInfo,
    mut secondary: AutoClientInfo,
    app: AppHandle,
) -> anyhow::Result<()> {
    let _status_guard = ClientStatusOfflineGuard { app: app.clone() };
    let log = Logger::new(&app);
    // The secondary's LoginStart was already drained at pairing time in
    // `run_automatic_mode` (so its socket could be watched for disconnect while
    // it waited); only the primary's remains to be read here.
    let primary_login_start = RawPacket::read_async(&mut primary.stream).await?;

    if primary.protocol_version != secondary.protocol_version {
        crate::proxy::send_login_error(
            &mut primary.stream,
            &mut secondary.stream,
            "Версии клиентов различаются".to_string(),
        )
        .await;
        return Ok(());
    }

    let version = match Version::from_protocol(primary.protocol_version) {
        Some(v) => v,
        None => {
            crate::proxy::send_login_error(
                &mut primary.stream,
                &mut secondary.stream,
                "Данная версия не поддерживается".to_string(),
            )
            .await;
            return Ok(());
        }
    };

    let remote_addr = match resolve_host_port(
        &secondary.server_host,
        secondary.server_port,
        "minecraft",
        "tcp",
    )
    .await
    {
        Some(a) => a,
        None => {
            crate::proxy::send_login_error(
                &mut primary.stream,
                &mut secondary.stream,
                format!("Не удалось разрешить адрес: {}", secondary.server_host),
            )
            .await;
            return Ok(());
        }
    };

    let mut remote_stream = match TcpStream::connect(remote_addr).await {
        Ok(s) => s,
        Err(_) => {
            crate::proxy::send_login_error(
                &mut primary.stream,
                &mut secondary.stream,
                "Ошибка при подключении к удалённому серверу".to_string(),
            )
            .await;
            return Ok(());
        }
    };
    log.success(format!("Подключено к {}", remote_addr));

    let handshake = Handshake {
        protocol_version: VarInt(primary.protocol_version),
        server_address: secondary.server_host.clone(),
        server_port: secondary.server_port,
        intent: Intent::Login.into(),
    };
    UncompressedPacket::from_packet(&handshake)?
        .write_async(&mut remote_stream)
        .await?;

    primary_login_start.write_async(&mut remote_stream).await?;
    let nickname = version
        .parse_login_start(&primary_login_start)
        .unwrap_or("...".to_string());
    NickNameEvent(nickname.clone()).emit(&app).ok();
    ServerAddrEvent(handshake.server_address.clone())
        .emit(&app)
        .ok();
    tokio::spawn(config::send_join(
        handshake.server_address.clone(),
        nickname,
        handshake.protocol_version.0,
    ));

    crate::proxy::run_proxy_session(
        primary.stream,
        secondary.stream,
        remote_stream,
        version,
        app,
    )
    .await
}

pub async fn run_panic_mode(client: AutoClientInfo) -> anyhow::Result<()> {
    let remote_addr = match resolve_host_port(
        &client.server_host,
        client.server_port,
        "minecraft",
        "tcp",
    )
    .await
    {
        Some(a) => a,
        None => {
            return Ok(());
        }
    };

    let mut remote_stream = TcpStream::connect(remote_addr).await?;

    let handshake = Handshake {
        protocol_version: VarInt(client.protocol_version),
        server_address: client.server_host.clone(),
        server_port: client.server_port,
        intent: Intent::Login.into(),
    };
    UncompressedPacket::from_packet(&handshake)?
        .write_async(&mut remote_stream)
        .await?;

    async fn proxy(mut read: OwnedReadHalf, mut write: OwnedWriteHalf) -> anyhow::Result<()> {
        loop {
            let packet = RawPacket::read_async(&mut read).await?;
            packet.write_async(&mut write).await?;
        }
    }

    let (client_read, client_write) = client.stream.into_split();
    let (remote_read, remote_write) = remote_stream.into_split();

    tokio::select! {
        res = proxy(client_read, remote_write) => res,
        res = proxy(remote_read, client_write) => res,
    }?;

    Ok(())
}

#[cfg(target_os = "windows")]
pub async fn run_automatic_mode(
    use_windivert: bool,
    port_min: u16,
    port_max: u16,
    app: AppHandle,
    panic_mode: Arc<Mutex<bool>>,
) -> anyhow::Result<()> {
    let log = Logger::new(&app);
    use crate::hotspot_redirect;
    use std::sync::Arc;

    let _redirect_handle;

    if use_windivert {
        if !hotspot_redirect::is_admin() {
            anyhow::bail!("Автоматический режим требует прав администратора.");
        }

        let (nat_table, redirect) =
            match hotspot_redirect::start_redirect(BIND_PORT, port_min, port_max, app.clone()) {
                Ok(t) => t,
                Err(e) => anyhow::bail!("WinDivert недоступен: {}", e),
            };
        hotspot_redirect::start_nat_cleanup(Arc::clone(&nat_table));
        _redirect_handle = Some(redirect);
        log.success("WinDivert перехват активен");
    } else {
        _redirect_handle = None;
        log.info("WinDivert отключён — подключайтесь напрямую");
    }

    let listener = match TcpListener::bind(format!("0.0.0.0:{}", BIND_PORT)).await {
        Ok(l) => l,
        Err(e) => anyhow::bail!("Ошибка при создании сокета: {}", e),
    };

    let (tx, mut rx) = mpsc::channel(HANDSHAKE_CHANNEL_CAPACITY);
    let mut session_set: JoinSet<anyhow::Result<()>> = JoinSet::new();
    session_set.spawn(async move {
        crate::proxy::listen_and_dispatch_auto(listener, tx).await;
        Ok(())
    });

    let mut pending: Option<AutoClientInfo> = None;

    loop {
        // Receive the next client; while a secondary is pending, also watch its
        // socket so we notice if it disconnects before the primary connects.
        enum Step {
            Client(Option<AutoClientInfo>),
            SecondaryGone,
        }
        let step = match pending.as_mut() {
            Some(sec) => tokio::select! {
                recv = rx.recv() => Step::Client(recv),
                _ = wait_for_disconnect(&mut sec.stream) => Step::SecondaryGone,
            },
            None => Step::Client(rx.recv().await),
        };
        let mut client = match step {
            Step::SecondaryGone => {
                log.warn("Второй клиент отключился, не дождавшись основного");
                ClientStatusEvent {
                    which: WhichClient::Secondary,
                    online: false,
                }
                .emit(&app)
                .ok();
                pending = None;
                continue;
            }
            Step::Client(Some(c)) => c,
            Step::Client(None) => break,
        };

        if *panic_mode.lock().await {
            tokio::spawn(run_panic_mode(client));
            continue;
        }
        if pending.is_none() && is_loopback_host(&client.server_host) {
            log.warn("Клиент подключился к 127.0.0.1 раньше второго клиента — отклонён");
            tokio::spawn(async move {
                // drain the client's LoginStart, then send the disconnect reason
                let _ = RawPacket::read_async(&mut client.stream).await;
                crate::proxy::send_login_disconnect(
                    &mut client.stream,
                    "Неправильный порядок подключения.\n\n\
                     Сначала зайдите на сервер на ВТОРОМ устройстве (дополнительный клиент \
                     через хотспот), и только потом — основным клиентом на 127.0.0.1.\n\n\
                     Похоже, настройка неверная. Прочитайте инструкцию в приложении VoxelProxy."
                        .to_string(),
                )
                .await;
            });
            continue;
        }
        match pending.take() {
            Some(secondary) => {
                ClientStatusEvent {
                    which: WhichClient::Primary,
                    online: true,
                }
                .emit(&app)
                .ok();
                session_set.spawn(run_auto_session(client, secondary, app.clone()));
            }
            None => {
                // Drain the secondary's LoginStart now (it is discarded anyway, see
                // run_auto_session) so wait_for_disconnect watches a quiet socket
                // instead of one with the buffered LoginStart still readable.
                let _ = RawPacket::read_async(&mut client.stream).await;
                ClientStatusEvent {
                    which: WhichClient::Secondary,
                    online: true,
                }
                .emit(&app)
                .ok();
                pending = Some(client);
            }
        }
    }

    Ok(())
}

#[cfg(not(target_os = "windows"))]
pub async fn run_automatic_mode(
    _use_windivert: bool,
    _port_min: u16,
    _port_max: u16,
    app: AppHandle,
    panic_mode: Arc<Mutex<bool>>,
) -> anyhow::Result<()> {
    let listener = TcpListener::bind(format!("0.0.0.0:{}", BIND_PORT)).await?;

    let (tx, mut rx) = mpsc::channel(HANDSHAKE_CHANNEL_CAPACITY);
    let mut session_set: JoinSet<anyhow::Result<()>> = JoinSet::new();
    session_set.spawn(async move {
        crate::proxy::listen_and_dispatch_auto(listener, tx).await;
        Ok(())
    });

    let mut pending: Option<AutoClientInfo> = None;

    loop {
        // Receive the next client; while a secondary is pending, also watch its
        // socket so we notice if it disconnects before the primary connects.
        enum Step {
            Client(Option<AutoClientInfo>),
            SecondaryGone,
        }
        let step = match pending.as_mut() {
            Some(sec) => tokio::select! {
                recv = rx.recv() => Step::Client(recv),
                _ = wait_for_disconnect(&mut sec.stream) => Step::SecondaryGone,
            },
            None => Step::Client(rx.recv().await),
        };
        let mut client = match step {
            Step::SecondaryGone => {
                ClientStatusEvent {
                    which: WhichClient::Secondary,
                    online: false,
                }
                .emit(&app)
                .ok();
                pending = None;
                continue;
            }
            Step::Client(Some(c)) => c,
            Step::Client(None) => break,
        };

        if *panic_mode.lock().await {
            tokio::spawn(run_panic_mode(client));
            continue;
        }
        match pending.take() {
            Some(secondary) => {
                ClientStatusEvent {
                    which: WhichClient::Primary,
                    online: true,
                }
                .emit(&app)
                .ok();
                session_set.spawn(run_auto_session(client, secondary, app.clone()));
            }
            None => {
                // Drain the secondary's LoginStart now (it is discarded anyway, see
                // run_auto_session) so wait_for_disconnect watches a quiet socket
                // instead of one with the buffered LoginStart still readable.
                let _ = RawPacket::read_async(&mut client.stream).await;
                ClientStatusEvent {
                    which: WhichClient::Secondary,
                    online: true,
                }
                .emit(&app)
                .ok();
                pending = Some(client);
            }
        }
    }

    Ok(())
}
