use std::{
    net::{Ipv4Addr, SocketAddr},
    process::Command,
};

use anyhow::anyhow;
use dialoguer::theme::ColorfulTheme;
use mc_protocol::{
    packet::{RawPacket, UncompressedPacket},
    varint::VarInt,
};
use serde_json::json;
use tokio::{
    io::AsyncWriteExt,
    net::{TcpListener, TcpStream},
    sync::mpsc::{self, Receiver, Sender},
};

use crate::{
    controller::{ClientId, Controller, run_client, run_server},
    local_ip::get_local_ip,
    packets::universal::{
        Intent,
        handshaking::c2s::Handshake,
        login::s2c::{EncryptionRequest, LoginDisconnect, LoginSuccess, SetCompression},
    },
    protocols::Version,
    resolver::resolve_host_port,
    updater::has_update,
};

const DEFAULT_PORT: u16 = 25565;
const BIND_PORT: u16 = 25565;
const HANDSHAKE_CHANNEL_CAPACITY: usize = 32;
const IO_CHANNEL_CAPACITY: usize = 100;

mod controller;
#[cfg(target_os = "windows")]
mod keybind;
mod local_ip;
#[allow(dead_code)]
mod packets;
mod protocols;
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
    #[cfg(target_os = "windows")]
    #[cfg(not(debug_assertions))]
    unsafe {
        use windows::Win32::System::Console::AllocConsole;

        AllocConsole().unwrap();
    }

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
            Ok(None) => {
                println!(" У вас последняя версия!");
            }
            Err(e) => {
                println!("При проверки обновлений произошла ошибка: {}", e);
                println!("Проверьте соединение к интернету");
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        use std::sync::mpsc;

        use crate::keybind::setup_keybind;
        let (tx, rx) = mpsc::channel();

        std::thread::spawn(|| unsafe { setup_keybind(tx) });
        println!("{}", rx.recv()?);
    }

    let (remote_addr, remote_dns) = {
        loop {
            let input: String = dialoguer::Input::with_theme(&ColorfulTheme::default())
                .with_prompt("Введите адрес сервера")
                .interact_text()?;

            if let Some(addr) = resolve_host_port(&input, DEFAULT_PORT, "minecraft", "tcp").await {
                break (addr, input);
            } else {
                println!("Ошибка");
            }
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

    let (tx, rx) = mpsc::channel(HANDSHAKE_CHANNEL_CAPACITY);

    let handler = tokio::spawn(handle_clients(rx, remote_addr, remote_dns));

    tokio::spawn(async move {
        while let Ok((stream, _addr)) = listener.accept().await {
            tokio::spawn(handle_connection(stream, tx.clone(), remote_addr.clone()));
        }
    });

    handler.await??;

    Ok(())
}

#[inline]
async fn read_uncompressed(stream: &mut TcpStream) -> anyhow::Result<UncompressedPacket> {
    Ok(RawPacket::read_async(stream).await?.as_uncompressed()?)
}

async fn handle_connection(
    mut stream: TcpStream,
    tx: Sender<(TcpStream, i32)>,
    remote_addr: SocketAddr,
) -> anyhow::Result<()> {
    let handshake: Handshake = read_uncompressed(&mut stream)
        .await?
        .deserialize_payload()?;

    match Intent::try_from(handshake.intent.0) {
        Ok(Intent::Status) => process_status(stream, remote_addr, handshake).await?,
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
    handshake: Handshake,
) -> anyhow::Result<()> {
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

async fn handle_clients(
    mut rx: Receiver<(TcpStream, i32)>,
    remote_addr: SocketAddr,
    remote_dns: String,
) -> anyhow::Result<()> {
    let (mut cheat_stream, cheat_protocol) = rx.recv().await.unwrap();
    let cheat_login_start = RawPacket::read_async(&mut cheat_stream).await?;
    println!("[+] Клиент с читами");

    let (mut legit_stream, legit_protocol) = rx.recv().await.unwrap();
    let _ = RawPacket::read_async(&mut legit_stream).await?;
    println!("[+] Клиент без читов");

    if cheat_protocol != legit_protocol {
        error_handler(
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
            error_handler(
                &mut cheat_stream,
                &mut legit_stream,
                "Данная версия не поддерживается".to_string(),
            )
            .await;
            return Ok(());
        }
    };

    println!("Подключение к {}", &remote_addr);
    let mut remote_stream = match TcpStream::connect(remote_addr).await {
        Ok(t) => t,
        Err(_) => {
            error_handler(
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
        server_address: remote_dns.clone(),
        server_port: DEFAULT_PORT,
        intent: Intent::Login.into(),
    };

    UncompressedPacket::from_packet(&handshake)?
        .write_async(&mut remote_stream)
        .await?;
    println!("[+] Handshake");

    cheat_login_start.write_async(&mut remote_stream).await?;
    println!("[+] Login start");

    let mut threshold = None;

    loop {
        let packet = RawPacket::read_async(&mut remote_stream)
            .await?
            .uncompress(threshold)?;

        match packet.packet_id {
            LoginDisconnect::PACKET_ID => {
                error_handler(
                    &mut cheat_stream,
                    &mut legit_stream,
                    packet.deserialize_payload::<LoginDisconnect>()?.reason,
                )
                .await;
                return Err(anyhow!("Disconnected"));
            }
            EncryptionRequest::PACKET_ID => {
                error_handler(
                    &mut cheat_stream,
                    &mut legit_stream,
                    "Лицензионный сервер не поддерживается\nИспользуйте ViaProxy".to_string(),
                )
                .await;
                return Err(anyhow!("Licensed"));
            }
            LoginSuccess::PACKET_ID => {
                let packet = packet.to_raw_packet_compressed(threshold)?;
                packet.write_async(&mut cheat_stream).await?;
                packet.write_async(&mut legit_stream).await?;
                println!("[+] Login success");
                break;
            }
            SetCompression::PACKET_ID => {
                let compression: SetCompression = packet.deserialize_payload()?;
                threshold = Some(compression.threshold.0);

                let packet = packet.to_raw_packet()?;
                packet.write_async(&mut cheat_stream).await?;
                packet.write_async(&mut legit_stream).await?;
                println!("[+] Compression");
            }
            _ => {
                unreachable!();
            }
        }
    }

    let (cheat_read, cheat_write) = cheat_stream.into_split();
    let (legit_read, legit_write) = legit_stream.into_split();
    let (remote_read, remote_write) = remote_stream.into_split();

    let (event_tx, event_rx) = mpsc::channel(IO_CHANNEL_CAPACITY);
    let (cheat_tx, cheat_rx) = mpsc::channel(IO_CHANNEL_CAPACITY);
    let (legit_tx, legit_rx) = mpsc::channel(IO_CHANNEL_CAPACITY);
    let (remote_tx, remote_rx) = mpsc::channel(IO_CHANNEL_CAPACITY);

    let controller = Controller::new(
        ClientId::Cheat,
        cheat_tx,
        legit_tx,
        remote_tx,
        event_rx,
        threshold,
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

    tokio::spawn(run_server(
        remote_read,
        remote_write,
        event_tx.clone(),
        remote_rx,
    ));

    println!("VoxelProxy запущен!");

    controller.run().await;
    Ok(())
}

async fn error_handler<W: AsyncWriteExt + Unpin>(
    cheat_stream: &mut W,
    legit_stream: &mut W,
    message: String,
) {
    let disconnect = UncompressedPacket::from_packet(&LoginDisconnect {
        reason: json!({"text": message}).to_string(),
    })
    .unwrap();

    disconnect.write_async(cheat_stream).await.unwrap();
    disconnect.write_async(legit_stream).await.unwrap();
}
