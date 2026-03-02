use std::{net::Ipv4Addr, process::Command};

use dialoguer::theme::ColorfulTheme;
use mc_protocol::{
    packet::{RawPacket, UncompressedPacket},
    varint::VarInt,
};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::mpsc,
};

use crate::{
    local_ip::get_local_ip,
    packets::universal::{Intent, handshaking::c2s::Handshake},
    protocols::Version,
    proxy::{BIND_PORT, DEFAULT_PORT, HANDSHAKE_CHANNEL_CAPACITY},
    resolver::resolve_host_port,
    updater::has_update,
};

mod controller;
#[cfg(target_os = "windows")]
#[cfg(not(debug_assertions))]
mod keybind;
mod local_ip;
#[allow(dead_code)]
mod packets;
mod protocols;
mod proxy;
mod resolver;
mod updater;

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        println!("Ошибка: {}", e);

        #[cfg(target_os = "windows")]
        #[cfg(not(debug_assertions))]
        unsafe {
            use windows::Win32::{
                System::Console::GetConsoleWindow,
                UI::WindowsAndMessaging::{SW_SHOW, ShowWindow},
            };
            let _ = ShowWindow(GetConsoleWindow(), SW_SHOW);
        }

        let _: String = dialoguer::Input::new().interact_text().unwrap();
    }
}

async fn run() -> anyhow::Result<()> {
    // ── Console allocation (Windows release builds) ───────────────────────────
    #[cfg(target_os = "windows")]
    #[cfg(not(debug_assertions))]
    unsafe {
        use windows::Win32::System::Console::AllocConsole;
        AllocConsole().unwrap();
    }

    print_banner();
    check_for_updates().await?;

    // ── Global keybind listener (Windows only) ────────────────────────────────
    #[cfg(target_os = "windows")]
    #[cfg(not(debug_assertions))]
    {
        use crate::keybind::setup_keybind;
        use std::sync::mpsc;

        let (tx, rx) = mpsc::channel();
        std::thread::spawn(|| unsafe { setup_keybind(tx) });
        println!("{}", rx.recv()?);
    }

    run_manual_mode().await

    // ── Mode selection ────────────────────────────────────────────────────────
    // let mode = dialoguer::Select::with_theme(&ColorfulTheme::default())
    //     .with_prompt("Выберите режим")
    //     .item("Ручной (ввести адрес сервера)")
    //     .item("Автоматический (WinDivert)")
    //     .default(0)
    //     .interact()?;

    // match mode {
    //     0 => run_manual_mode().await,
    //     1 => run_automatic_mode().await,
    //     _ => unreachable!(),
    // }
}

// ── Manual mode ───────────────────────────────────────────────────────────────

async fn run_manual_mode() -> anyhow::Result<()> {
    let (remote_addr, remote_dns) = loop {
        let input: String = dialoguer::Input::with_theme(&ColorfulTheme::default())
            .with_prompt("Введите адрес сервера")
            .interact_text()?;

        if let Some(addr) = resolve_host_port(&input, DEFAULT_PORT, "minecraft", "tcp").await {
            break (addr, input);
        } else {
            println!("Ошибка");
        }
    };

    let listener = match TcpListener::bind(format!("0.0.0.0:{}", BIND_PORT)).await {
        Ok(t) => t,
        Err(e) => {
            println!("Ошибка при создании сокета. {}", e);
            loop {
                let _: String = dialoguer::Input::new().interact_text()?;
            }
        }
    };

    let addr = get_local_ip().unwrap_or(Ipv4Addr::new(127, 0, 0, 1));
    println!("Адрес для подключения (сначала чит, потом легит): {}", addr);

    let (tx, mut rx) = mpsc::channel(HANDSHAKE_CHANNEL_CAPACITY);
    tokio::spawn(proxy::listen_and_dispatch(
        listener,
        tx,
        remote_dns.clone(),
        remote_addr,
    ));

    // ── Pair clients ──────────────────────────────────────────────────────────

    let (mut cheat_stream, cheat_protocol) = rx.recv().await.unwrap();
    let cheat_login_start = RawPacket::read_async(&mut cheat_stream).await?;
    println!("[+] Клиент с читами");

    let (mut legit_stream, legit_protocol) = rx.recv().await.unwrap();
    let _ = RawPacket::read_async(&mut legit_stream).await?;
    println!("[+] Клиент без читов");

    if cheat_protocol != legit_protocol {
        proxy::send_login_error(
            &mut cheat_stream,
            &mut legit_stream,
            "Версии клиентов различаются".to_string(),
        )
        .await;
        return Ok(());
    }

    let version = match Version::from_protocol(cheat_protocol) {
        Some(v) => v,
        None => {
            proxy::send_login_error(
                &mut cheat_stream,
                &mut legit_stream,
                "Данная версия не поддерживается".to_string(),
            )
            .await;
            return Ok(());
        }
    };

    // ── Connect to remote and perform login handshake ─────────────────────────

    println!("Подключение к {}", remote_addr);
    let mut remote_stream = match TcpStream::connect(remote_addr).await {
        Ok(t) => t,
        Err(_) => {
            proxy::send_login_error(
                &mut cheat_stream,
                &mut legit_stream,
                "Ошибка при подключении к удаленному серверу".to_string(),
            )
            .await;
            return Ok(());
        }
    };
    println!("Успех");

    let handshake = Handshake {
        protocol_version: VarInt(cheat_protocol),
        server_address: remote_dns,
        server_port: DEFAULT_PORT,
        intent: Intent::Login.into(),
    };
    UncompressedPacket::from_packet(&handshake)?
        .write_async(&mut remote_stream)
        .await?;
    println!("[+] Handshake");

    cheat_login_start.write_async(&mut remote_stream).await?;
    println!("[+] Login start");

    proxy::run_proxy_session(cheat_stream, legit_stream, remote_stream, version).await
}

// ── Automatic mode ────────────────────────────────────────────────────────────

#[cfg(target_os = "windows")]
async fn _run_automatic_mode() -> anyhow::Result<()> {
    // TODO: intercept outgoing handshake packets with WinDivert, extract
    // remote_addr + remote_dns + protocol_version, then call run_proxy_session.
    anyhow::bail!("Автоматический режим ещё не реализован.")
}

#[cfg(not(target_os = "windows"))]
async fn _run_automatic_mode() -> anyhow::Result<()> {
    anyhow::bail!("Автоматический режим поддерживается только на Windows (WinDivert).")
}

// ── Startup helpers ───────────────────────────────────────────────────────────

fn print_banner() {
    println!(
        r#"
__     __            _ ____
\ \   / /____  _____| |  _ \ _ __ _____  ___   _
 \ \ / / _ \ \/ / _ \ | |_) | '__/ _ \ \/ / | | |
  \ V / (_) >  <  __/ |  __/| | | (_) >  <| |_| |
   \_/ \___/_/\_\___|_|_|   |_|  \___/_/\_\\__, |
                                           |___/"#
    );

    let version = env!("CARGO_PKG_VERSION");
    if cfg!(debug_assertions) {
        println!(" Версия: DEV v{}", version);
    } else {
        println!(" Версия: v{}", version);
    }
}

async fn check_for_updates() -> anyhow::Result<()> {
    if cfg!(debug_assertions) {
        return Ok(());
    }

    let version = env!("CARGO_PKG_VERSION");
    match has_update(version).await {
        Ok(Some(new_version)) => {
            println!(
                " Доступна новая версия, пожалуйста обновитесь: {}",
                &new_version.tag
            );
            println!(" Ссылка: {}", &new_version.link);

            #[cfg(target_os = "windows")]
            let _ = Command::new("cmd")
                .arg("/C")
                .arg("start")
                .arg(&new_version.link)
                .output();

            loop {
                let _: String = dialoguer::Input::new().interact_text()?;
            }
        }
        Ok(None) => println!(" У вас последняя версия!"),
        Err(e) => {
            println!("При проверки обновлений произошла ошибка: {}", e);
            println!("Проверьте соединение к интернету");
        }
    }

    Ok(())
}
