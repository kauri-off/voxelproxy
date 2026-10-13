#[cfg(target_os = "windows")]
use std::net::{IpAddr, ToSocketAddrs};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use mc_protocol::{packet::UncompressedPacket, varint::VarInt};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::mpsc,
    task::JoinSet,
};

use crate::{
    config,
    events::{AppEvent, EventSink, WhichClient},
    logger::Logger,
    packets::universal::{Intent, handshaking::c2s::Handshake},
    protocols::{Version, VersionProtocol},
    proxy::{BIND_PORT, DEFAULT_PORT, HANDSHAKE_CHANNEL_CAPACITY, LoginClient, connect},
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

/// Resolves when an idle post-login client disconnects; `peek` keeps it cancellation-safe.
async fn wait_for_disconnect(stream: &mut TcpStream) {
    let mut probe = [0u8; 1];
    let _ = stream.peek(&mut probe).await;
}

/// Marks both clients offline on any session exit, including early errors.
struct ClientStatusOfflineGuard {
    events: EventSink,
}

impl Drop for ClientStatusOfflineGuard {
    fn drop(&mut self) {
        for which in [WhichClient::Primary, WhichClient::Secondary] {
            self.events.client_status(which, false);
        }
    }
}

pub async fn run_manual_mode(server_addr: String, events: EventSink) -> anyhow::Result<()> {
    let _status_guard = ClientStatusOfflineGuard {
        events: events.clone(),
    };
    let log = Logger::new(&events);
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

    // Re-acquire the primary if it drops while waiting for the secondary.
    let (
        mut primary_stream,
        primary_protocol,
        primary_login_start,
        mut secondary_stream,
        secondary_protocol,
    ) = loop {
        let LoginClient {
            stream: mut primary_stream,
            protocol_version: primary_protocol,
            login_start: primary_login_start,
            ..
        } = match rx.recv().await {
            Some(p) => p,
            None => anyhow::bail!("Диспетчер подключений завершился"),
        };
        events.client_status(WhichClient::Primary, true);

        enum Sec {
            Got(LoginClient),
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
            Sec::Got(secondary) => {
                break (
                    primary_stream,
                    primary_protocol,
                    primary_login_start,
                    secondary.stream,
                    secondary.protocol_version,
                );
            }
            Sec::PrimaryGone => {
                log.warn("Основной клиент отключился, не дождавшись второго");
                events.client_status(WhichClient::Primary, false);
                continue;
            }
            Sec::Closed => anyhow::bail!("Диспетчер подключений завершился"),
        }
    };
    events.client_status(WhichClient::Secondary, true);

    if primary_protocol != secondary_protocol {
        crate::proxy::send_login_error(
            &mut primary_stream,
            &mut secondary_stream,
            "Версии клиентов различаются".to_string(),
            None,
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
                None,
            )
            .await;
            anyhow::bail!("Данная версия не поддерживается");
        }
    };

    let mut remote_stream = match connect(remote_addr).await {
        Ok(t) => t,
        Err(_) => {
            crate::proxy::send_login_error(
                &mut primary_stream,
                &mut secondary_stream,
                "Ошибка при подключении к удалённому серверу".to_string(),
                None,
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
    events.emit(AppEvent::NickName(nickname.clone()));
    events.emit(AppEvent::ServerAddr(handshake.server_address.clone()));
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
        events,
    )
    .await
}

async fn run_auto_session(
    mut primary: LoginClient,
    mut secondary: LoginClient,
    events: EventSink,
) -> anyhow::Result<()> {
    let _status_guard = ClientStatusOfflineGuard {
        events: events.clone(),
    };
    let log = Logger::new(&events);
    // Only the primary's LoginStart is forwarded; the secondary's is discarded.
    let primary_login_start = primary.login_start;

    if primary.protocol_version != secondary.protocol_version {
        crate::proxy::send_login_error(
            &mut primary.stream,
            &mut secondary.stream,
            "Версии клиентов различаются".to_string(),
            None,
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
                None,
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
                None,
            )
            .await;
            return Ok(());
        }
    };

    let mut remote_stream = match connect(remote_addr).await {
        Ok(s) => s,
        Err(_) => {
            crate::proxy::send_login_error(
                &mut primary.stream,
                &mut secondary.stream,
                "Ошибка при подключении к удалённому серверу".to_string(),
                None,
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
    events.emit(AppEvent::NickName(nickname.clone()));
    events.emit(AppEvent::ServerAddr(handshake.server_address.clone()));
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
        events,
    )
    .await
}

pub async fn run_panic_mode(mut client: LoginClient) -> anyhow::Result<()> {
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

    let mut remote_stream = connect(remote_addr).await?;

    let handshake = Handshake {
        protocol_version: VarInt(client.protocol_version),
        server_address: client.server_host.clone(),
        server_port: client.server_port,
        intent: Intent::Login.into(),
    };
    UncompressedPacket::from_packet(&handshake)?
        .write_async(&mut remote_stream)
        .await?;

    client.login_start.write_async(&mut remote_stream).await?;

    tokio::io::copy_bidirectional(&mut client.stream, &mut remote_stream).await?;
    Ok(())
}

#[cfg(target_os = "windows")]
pub async fn run_auto_mode(
    use_windivert: bool,
    port_min: u16,
    port_max: u16,
    events: EventSink,
    panic_mode: Arc<AtomicBool>,
) -> anyhow::Result<()> {
    let log = Logger::new(&events);
    use crate::hotspot_redirect;

    let _redirect_handle;

    if use_windivert {
        if !crate::backend::is_elevated() {
            anyhow::bail!("Режим «Через раздачу Wi-Fi» требует прав администратора.");
        }

        let redirect =
            match hotspot_redirect::start_redirect(BIND_PORT, port_min, port_max, events.clone()) {
                Ok(r) => r,
                Err(e) => anyhow::bail!("WinDivert недоступен: {}", e),
            };
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

    let mut pending: Option<LoginClient> = None;

    loop {
        // Receive the next client; while a secondary is pending, also watch its
        // socket so we notice if it disconnects before the primary connects.
        enum Step {
            Client(Option<LoginClient>),
            SecondaryGone,
            Finished(Result<anyhow::Result<()>, tokio::task::JoinError>),
        }
        let step = match pending.as_mut() {
            Some(sec) => tokio::select! {
                recv = rx.recv() => Step::Client(recv),
                _ = wait_for_disconnect(&mut sec.stream) => Step::SecondaryGone,
                Some(res) = session_set.join_next() => Step::Finished(res),
            },
            None => tokio::select! {
                recv = rx.recv() => Step::Client(recv),
                Some(res) = session_set.join_next() => Step::Finished(res),
            },
        };
        let mut client = match step {
            Step::Finished(Ok(Err(e))) => {
                log.error(e.to_string());
                continue;
            }
            Step::Finished(_) => continue,
            Step::SecondaryGone => {
                log.warn("Второй клиент отключился, не дождавшись основного");
                events.client_status(WhichClient::Secondary, false);
                pending = None;
                continue;
            }
            Step::Client(Some(c)) => c,
            Step::Client(None) => break,
        };

        if panic_mode.load(Ordering::Relaxed) {
            session_set.spawn(run_panic_mode(client));
            continue;
        }
        if pending.is_none() && is_loopback_host(&client.server_host) {
            log.warn("Клиент подключился к 127.0.0.1 раньше второго клиента — отклонён");
            session_set.spawn(async move {
                crate::proxy::send_login_disconnect(
                    &mut client.stream,
                    "Неправильный порядок подключения.\n\n\
                     Сначала зайдите на сервер на ВТОРОМ устройстве (дополнительный клиент \
                     через хотспот), и только потом — основным клиентом на 127.0.0.1.\n\n\
                     Похоже, настройка неверная. Прочитайте инструкцию в приложении VoxelProxy."
                        .to_string(),
                )
                .await;
                Ok(())
            });
            continue;
        }
        match pending.take() {
            Some(secondary) => {
                events.client_status(WhichClient::Primary, true);
                session_set.spawn(run_auto_session(client, secondary, events.clone()));
            }
            None => {
                events.client_status(WhichClient::Secondary, true);
                pending = Some(client);
            }
        }
    }

    Ok(())
}

#[cfg(not(target_os = "windows"))]
pub async fn run_auto_mode(
    _use_windivert: bool,
    _port_min: u16,
    _port_max: u16,
    events: EventSink,
    panic_mode: Arc<AtomicBool>,
) -> anyhow::Result<()> {
    let listener = TcpListener::bind(format!("0.0.0.0:{}", BIND_PORT)).await?;

    let (tx, mut rx) = mpsc::channel(HANDSHAKE_CHANNEL_CAPACITY);
    let mut session_set: JoinSet<anyhow::Result<()>> = JoinSet::new();
    session_set.spawn(async move {
        crate::proxy::listen_and_dispatch_auto(listener, tx).await;
        Ok(())
    });

    let mut pending: Option<LoginClient> = None;

    loop {
        while session_set.try_join_next().is_some() {}

        // Receive the next client; while a secondary is pending, also watch its
        // socket so we notice if it disconnects before the primary connects.
        enum Step {
            Client(Option<LoginClient>),
            SecondaryGone,
        }
        let step = match pending.as_mut() {
            Some(sec) => tokio::select! {
                recv = rx.recv() => Step::Client(recv),
                _ = wait_for_disconnect(&mut sec.stream) => Step::SecondaryGone,
            },
            None => Step::Client(rx.recv().await),
        };
        let client = match step {
            Step::SecondaryGone => {
                events.client_status(WhichClient::Secondary, false);
                pending = None;
                continue;
            }
            Step::Client(Some(c)) => c,
            Step::Client(None) => break,
        };

        if panic_mode.load(Ordering::Relaxed) {
            session_set.spawn(run_panic_mode(client));
            continue;
        }
        match pending.take() {
            Some(secondary) => {
                events.client_status(WhichClient::Primary, true);
                session_set.spawn(run_auto_session(client, secondary, events.clone()));
            }
            None => {
                events.client_status(WhichClient::Secondary, true);
                pending = Some(client);
            }
        }
    }

    Ok(())
}
