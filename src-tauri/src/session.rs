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

pub async fn run_manual_mode(server_addr: String, app: AppHandle) -> anyhow::Result<()> {
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

    let (mut primary_stream, primary_protocol) = rx.recv().await.unwrap();
    let primary_login_start = RawPacket::read_async(&mut primary_stream).await?;

    ClientStatusEvent {
        which: WhichClient::Primary,
        online: true,
    }
    .emit(&app)
    .ok();

    let (mut secondary_stream, secondary_protocol) = rx.recv().await.unwrap();
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
    let log = Logger::new(&app);
    let primary_login_start = RawPacket::read_async(&mut primary.stream).await?;
    let _ = RawPacket::read_async(&mut secondary.stream).await?;

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

    let mut pending = Vec::new();

    while let Some(client) = rx.recv().await {
        if *panic_mode.lock().await {
            tokio::spawn(run_panic_mode(client));
            continue;
        }
        if let Some(secondary) = pending.pop() {
            ClientStatusEvent {
                which: WhichClient::Primary,
                online: true,
            }
            .emit(&app)
            .ok();
            session_set.spawn(run_auto_session(client, secondary, app.clone()));
        } else {
            ClientStatusEvent {
                which: WhichClient::Secondary,
                online: true,
            }
            .emit(&app)
            .ok();
            pending.push(client);
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

    let mut pending = Vec::new();

    while let Some(client) = rx.recv().await {
        if *panic_mode.lock().await {
            tokio::spawn(run_panic_mode(client));
            continue;
        }
        if let Some(secondary) = pending.pop() {
            ClientStatusEvent {
                which: WhichClient::Primary,
                online: true,
            }
            .emit(&app)
            .ok();
            session_set.spawn(run_auto_session(client, secondary, app.clone()));
        } else {
            ClientStatusEvent {
                which: WhichClient::Secondary,
                online: true,
            }
            .emit(&app)
            .ok();
            pending.push(client);
        }
    }

    Ok(())
}
