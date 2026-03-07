use std::{net::Ipv4Addr, sync::Arc};

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
    local_ip::get_local_ip,
    logger::Logger,
    packets::universal::{Intent, handshaking::c2s::Handshake},
    protocols::Version,
    proxy::{BIND_PORT, DEFAULT_PORT, HANDSHAKE_CHANNEL_CAPACITY},
    resolver::resolve_host_port,
};

// ── Manual mode ───────────────────────────────────────────────────────────────

pub async fn run_manual_mode(server_addr: String, log: Logger) -> anyhow::Result<()> {
    let (remote_addr, remote_dns) =
        match resolve_host_port(&server_addr, DEFAULT_PORT, "minecraft", "tcp").await {
            Some(addr) => (addr, server_addr),
            None => anyhow::bail!("Не удалось разрешить адрес: \"{}\"", server_addr),
        };

    let listener = match TcpListener::bind(format!("0.0.0.0:{}", BIND_PORT)).await {
        Ok(t) => t,
        Err(e) => anyhow::bail!("Ошибка при создании сокета: {}", e),
    };

    let addr = get_local_ip().unwrap_or(Ipv4Addr::new(127, 0, 0, 1));
    log.info(format!("Ожидание подключений на {}:{}", addr, BIND_PORT));
    log.info("Порядок: сначала основной клиент, потом дополнительный");

    let (tx, mut rx) = mpsc::channel(HANDSHAKE_CHANNEL_CAPACITY);
    // Use JoinSet so the listener task is automatically aborted when this
    // function returns or is cancelled (releasing the bound port).
    let mut _dispatch_set: JoinSet<()> = JoinSet::new();
    _dispatch_set.spawn(crate::proxy::listen_and_dispatch(
        listener,
        tx,
        remote_dns.clone(),
        remote_addr,
    ));

    let (mut primary_stream, primary_protocol) = rx.recv().await.unwrap();
    let primary_login_start = RawPacket::read_async(&mut primary_stream).await?;
    log.success("Основной клиент подключён");
    log.client_status("primary", true);

    let (mut secondary_stream, secondary_protocol) = rx.recv().await.unwrap();
    let _ = RawPacket::read_async(&mut secondary_stream).await?;
    log.success("Дополнительный клиент подключён");
    log.client_status("secondary", true);

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

    log.info(format!("Подключение к {}...", remote_addr));
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
    log.info("Handshake отправлен");

    primary_login_start.write_async(&mut remote_stream).await?;
    log.info("Login Start отправлен");

    log.success("VoxelProxy запущен!");
    crate::proxy::run_proxy_session(
        primary_stream,
        secondary_stream,
        remote_stream,
        version,
        log.clone(),
    )
    .await
}

// ── Automatic mode (Windows only) ─────────────────────────────────────────────

#[cfg(target_os = "windows")]
pub async fn run_automatic_mode(log: Logger) -> anyhow::Result<()> {
    use crate::hotspot_redirect;

    if !hotspot_redirect::is_admin() {
        anyhow::bail!("Автоматический режим требует прав администратора.");
    }

    let nat_table = match hotspot_redirect::start_redirect(BIND_PORT, log.clone()) {
        Ok(t) => t,
        Err(e) => anyhow::bail!("WinDivert недоступен: {}", e),
    };
    hotspot_redirect::start_nat_cleanup(Arc::clone(&nat_table));
    log.success("WinDivert перехват активен");

    let listener = match TcpListener::bind(format!("0.0.0.0:{}", BIND_PORT)).await {
        Ok(l) => l,
        Err(e) => anyhow::bail!("Ошибка при создании сокета: {}", e),
    };
    log.info(format!("Ожидание подключений на порту {}", BIND_PORT));
    log.info("Порты 25560–25570 перехватываются WinDivert");
    log.info("Порядок: сначала дополнительный клиент, затем основной");
    log.info(format!(
        "Основной клиент подключайтесь к 127.0.0.1:{}",
        BIND_PORT
    ));

    let (tx, mut rx) = mpsc::channel(HANDSHAKE_CHANNEL_CAPACITY);
    // JoinSet: dropping it aborts the listener + all active session tasks,
    // which releases the bound port when the mode is stopped.
    let mut session_set: JoinSet<anyhow::Result<()>> = JoinSet::new();
    session_set.spawn(async move {
        crate::proxy::listen_and_dispatch_auto(listener, tx).await;
        Ok(())
    });

    let mut pending = Vec::new();

    while let Some(client) = rx.recv().await {
        if let Some(secondary) = pending.pop() {
            log.client_status("primary", true);
            session_set.spawn(run_auto_session(client, secondary, log.clone()));
        } else {
            let key = (client.server_host.clone(), client.server_port);
            log.client_status("secondary", true);
            log.info(format!(
                "Дополнительный клиент подключён ({}:{}), ожидание основного...",
                key.0, key.1
            ));
            pending.push(client);
        }
    }

    Ok(())
}

#[cfg(target_os = "windows")]
async fn run_auto_session(
    mut primary: crate::proxy::AutoClientInfo,
    mut secondary: crate::proxy::AutoClientInfo,
    log: Logger,
) -> anyhow::Result<()> {
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

    log.info(format!("Подключение к {}...", remote_addr));
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
    log.success("VoxelProxy сессия запущена!");

    crate::proxy::run_proxy_session(
        primary.stream,
        secondary.stream,
        remote_stream,
        version,
        log.clone(),
    )
    .await
}
