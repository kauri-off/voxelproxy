use std::fmt;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use mc_protocol::{
    packet::{RawPacket, UncompressedPacket},
    varint::VarInt,
};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::mpsc,
    task::JoinSet,
};

use crate::{
    config,
    events::{AppEvent, ClientId, EventSink},
    logger::Logger,
    packets::universal::{Intent, handshaking::c2s::Handshake},
    protocols::{Version, VersionProtocol},
    proxy::{BIND_PORT, HANDSHAKE_CHANNEL_CAPACITY, LoginClient, connect},
    resolver::{resolve_host_port, resolve_server},
};

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
        for which in [ClientId::Primary, ClientId::Secondary] {
            self.events.client_status(which, false);
        }
    }
}

/// A pair rejected with a LoginDisconnect; auto mode treats it as a normal session end.
#[derive(Debug)]
struct Rejected(String);

impl fmt::Display for Rejected {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Rejected {}

async fn reject(
    primary: &mut LoginClient,
    secondary: &mut LoginClient,
    reason: String,
) -> anyhow::Error {
    crate::proxy::send_login_error(
        &mut primary.stream,
        &mut secondary.stream,
        reason.clone(),
        None,
    )
    .await;
    Rejected(reason).into()
}

/// Connects to the server and replays the client's Handshake and LoginStart.
async fn open_upstream(
    addr: SocketAddr,
    host: String,
    port: u16,
    protocol: i32,
    login_start: &RawPacket,
) -> anyhow::Result<TcpStream> {
    let mut stream = connect(addr).await?;
    let handshake = Handshake {
        protocol_version: VarInt(protocol),
        server_address: host,
        server_port: port,
        intent: Intent::Login.into(),
    };
    UncompressedPacket::from_packet(&handshake)?
        .write_async(&mut stream)
        .await?;
    login_start.write_async(&mut stream).await?;
    Ok(stream)
}

/// Where a pair connects; `addr` is resolved from `host`/`port` when absent.
struct Upstream {
    host: String,
    port: u16,
    addr: Option<SocketAddr>,
    server_addr: String,
}

async fn start_pair(
    mut primary: LoginClient,
    mut secondary: LoginClient,
    upstream: Upstream,
    events: EventSink,
) -> anyhow::Result<()> {
    let log = Logger::new(&events);

    if primary.protocol_version != secondary.protocol_version {
        let reason = "Версии клиентов различаются".to_string();
        return Err(reject(&mut primary, &mut secondary, reason).await);
    }

    let Some(version) = Version::from_protocol(primary.protocol_version) else {
        let reason = "Данная версия не поддерживается".to_string();
        return Err(reject(&mut primary, &mut secondary, reason).await);
    };

    let addr = match upstream.addr {
        Some(addr) => addr,
        None => match resolve_host_port(&upstream.host, upstream.port).await {
            Some(t) => t.addr,
            None => {
                let reason = format!("Не удалось разрешить адрес: {}", upstream.host);
                return Err(reject(&mut primary, &mut secondary, reason).await);
            }
        },
    };

    // Only the primary's LoginStart is forwarded; the secondary's is discarded.
    let Ok(remote_stream) = open_upstream(
        addr,
        upstream.host,
        upstream.port,
        primary.protocol_version,
        &primary.login_start,
    )
    .await
    else {
        let reason = "Ошибка при подключении к удалённому серверу".to_string();
        return Err(reject(&mut primary, &mut secondary, reason).await);
    };
    log.success(format!("Подключено к {}", addr));

    let nickname = version
        .parse_login_start(&primary.login_start)
        .unwrap_or("...".to_string());
    events.emit(AppEvent::NickName(nickname.clone()));
    events.emit(AppEvent::ServerAddr(upstream.server_addr.clone()));
    tokio::spawn(config::send_join(
        upstream.server_addr,
        nickname,
        primary.protocol_version,
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

pub async fn run_manual_mode(server_addr: String, events: EventSink) -> anyhow::Result<()> {
    let _status_guard = ClientStatusOfflineGuard {
        events: events.clone(),
    };
    let log = Logger::new(&events);
    let Some(target) = resolve_server(&server_addr).await else {
        anyhow::bail!("Не удалось разрешить адрес: \"{}\"", server_addr);
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
        target.clone(),
    ));

    // Re-acquire the primary if it drops while waiting for the secondary.
    let (primary, secondary) = loop {
        let mut primary = match rx.recv().await {
            Some(p) => p,
            None => anyhow::bail!("Диспетчер подключений завершился"),
        };
        events.client_status(ClientId::Primary, true);

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
            _ = wait_for_disconnect(&mut primary.stream) => Sec::PrimaryGone,
        };
        match sec {
            Sec::Got(secondary) => break (primary, secondary),
            Sec::PrimaryGone => {
                log.warn("Основной клиент отключился, не дождавшись второго");
                events.client_status(ClientId::Primary, false);
                continue;
            }
            Sec::Closed => anyhow::bail!("Диспетчер подключений завершился"),
        }
    };
    events.client_status(ClientId::Secondary, true);

    let upstream = Upstream {
        host: target.host,
        port: target.port,
        addr: Some(target.addr),
        server_addr,
    };
    start_pair(primary, secondary, upstream, events).await
}

async fn run_auto_session(
    primary: LoginClient,
    secondary: LoginClient,
    events: EventSink,
) -> anyhow::Result<()> {
    let _status_guard = ClientStatusOfflineGuard {
        events: events.clone(),
    };
    let upstream = Upstream {
        host: secondary.server_host.clone(),
        port: secondary.server_port,
        addr: None,
        server_addr: secondary.server_host.clone(),
    };
    match start_pair(primary, secondary, upstream, events).await {
        Err(e) if e.is::<Rejected>() => Ok(()),
        res => res,
    }
}

pub async fn run_check_mode(mut client: LoginClient) -> anyhow::Result<()> {
    let Some(target) = resolve_host_port(&client.server_host, client.server_port).await else {
        return Ok(());
    };

    let mut remote_stream = open_upstream(
        target.addr,
        client.server_host.clone(),
        client.server_port,
        client.protocol_version,
        &client.login_start,
    )
    .await?;

    tokio::io::copy_bidirectional(&mut client.stream, &mut remote_stream).await?;
    Ok(())
}

pub async fn run_auto_mode(
    use_windivert: bool,
    port_min: u16,
    port_max: u16,
    events: EventSink,
    check_mode: Arc<AtomicBool>,
) -> anyhow::Result<()> {
    let log = Logger::new(&events);

    #[cfg(target_os = "windows")]
    let _redirect_handle = if use_windivert {
        if !crate::backend::is_elevated() {
            anyhow::bail!("Режим «Через раздачу Wi-Fi» требует прав администратора.");
        }

        let redirect = match crate::hotspot_redirect::start_redirect(
            BIND_PORT,
            port_min,
            port_max,
            events.clone(),
        ) {
            Ok(r) => r,
            Err(e) => anyhow::bail!("WinDivert недоступен: {}", e),
        };
        log.success("WinDivert перехват активен");
        Some(redirect)
    } else {
        log.info("WinDivert отключён — подключайтесь напрямую");
        None
    };
    #[cfg(not(target_os = "windows"))]
    let _ = (use_windivert, port_min, port_max);

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
        let client = match step {
            Step::Finished(Ok(Err(e))) => {
                log.error(e.to_string());
                continue;
            }
            Step::Finished(_) => continue,
            Step::SecondaryGone => {
                log.warn("Второй клиент отключился, не дождавшись основного");
                events.client_status(ClientId::Secondary, false);
                pending = None;
                continue;
            }
            Step::Client(Some(c)) => c,
            Step::Client(None) => break,
        };

        if check_mode.load(Ordering::Relaxed) {
            session_set.spawn(run_check_mode(client));
            continue;
        }
        #[cfg(target_os = "windows")]
        if pending.is_none() && client.is_loopback {
            log.warn("Клиент подключился к 127.0.0.1 раньше второго клиента — отклонён");
            let mut stream = client.stream;
            session_set.spawn(async move {
                crate::proxy::send_login_disconnect(
                    &mut stream,
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
                events.client_status(ClientId::Primary, true);
                session_set.spawn(run_auto_session(client, secondary, events.clone()));
            }
            None => {
                events.client_status(ClientId::Secondary, true);
                pending = Some(client);
            }
        }
    }

    Ok(())
}
