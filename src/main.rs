use std::{
    net::{Ipv4Addr, SocketAddr},
    process::Command,
};

use anyhow::anyhow;
use dialoguer::theme::ColorfulTheme;
use minecraft_protocol::{packet::RawPacket, varint::VarInt};
use serde_json::json;
use tokio::{
    io::AsyncWriteExt,
    net::{TcpListener, TcpStream},
    sync::mpsc::{self, Receiver, Sender},
};

use crate::{
    controller::{run_client, run_server, ClientId, Controller},
    local_ip::get_local_ip,
    packets::p767::{c2s, s2c},
    resolver::resolve_host_port,
    updater::has_update,
};

mod controller;
mod local_ip;
#[allow(dead_code)]
mod packets;
mod resolver;
mod updater;

#[tokio::main]
async fn main() {
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

                let _ = Command::new("cmd")
                    .arg("/C")
                    .arg("start")
                    .arg(&new_version.link)
                    .output();
                loop {
                    let _: String = dialoguer::Input::new().interact_text().unwrap();
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

    let (remote_addr, remote_dns) = {
        loop {
            let input: String = dialoguer::Input::with_theme(&ColorfulTheme::default())
                .with_prompt("Введите адрес сервера")
                .interact_text()
                .unwrap();

            if let Some(addr) = resolve_host_port(&input, 25565, "minecraft", "tcp").await {
                break (addr, input);
            } else {
                println!("Ошибка");
            }
        }
    };

    let listener = match TcpListener::bind("0.0.0.0:25565").await {
        Ok(t) => t,
        Err(e) => {
            println!("Ошибка при создании сокета. {}", e);
            loop {
                let _: String = dialoguer::Input::new().interact_text().unwrap();
            }
        }
    };

    let addr = get_local_ip().unwrap_or(Ipv4Addr::new(127, 0, 0, 1));

    println!("Адрес для подключения (сначала чит, потом легит): {}", addr);

    let (tx, rx) = mpsc::channel(32);

    let handler = tokio::spawn(handle_clients(rx, remote_addr, remote_dns));

    tokio::spawn(async move {
        while let Ok((stream, _addr)) = listener.accept().await {
            tokio::spawn(handle_connection(stream, tx.clone()));
        }
    });

    if let Err(e) = handler.await.unwrap() {
        println!("Ошибка: {}", e);
        loop {
            let _: String = dialoguer::Input::new().interact_text().unwrap();
        }
    }
}

async fn handle_connection(
    mut stream: TcpStream,
    tx: Sender<(TcpStream, i32)>,
) -> anyhow::Result<()> {
    let handshake: c2s::Handshake = RawPacket::read(&mut stream)
        .await?
        .as_uncompressed()?
        .convert()?;

    match handshake.intent.0 {
        1 => process_status(stream, handshake.protocol_version.0).await?,
        2 => {
            tx.send((stream, handshake.protocol_version.0)).await?;
        }
        _ => {}
    }

    Ok(())
}

async fn process_status(mut stream: TcpStream, _protocol: i32) -> anyhow::Result<()> {
    while let Ok(packet) = RawPacket::read(&mut stream).await {
        let packet_id = packet.clone().as_uncompressed()?.packet_id;

        if packet_id.0 == 0 {
            s2c::StatusResponse {
                response: json!({
                  "version": {
                    "name": "1.16.5",
                    "protocol": 754
                  },
                  "players": {
                    "max": 20,
                    "online": 0
                  },
                  "description": "A Minecraft Server",
                })
                .to_string(),
            }
            .as_uncompressed()?
            .to_raw_packet()?
            .write(&mut stream)
            .await?;
        } else if packet_id.0 == 1 {
            packet.write(&mut stream).await?;
        }
    }

    Ok(())
}

async fn handle_clients(
    mut rx: Receiver<(TcpStream, i32)>,
    remote_addr: SocketAddr,
    remote_dns: String,
) -> anyhow::Result<()> {
    let (mut cheat_stream, cheat_protocol) = rx.recv().await.unwrap();
    let cheat_login_start: c2s::LoginStart = RawPacket::read(&mut cheat_stream)
        .await?
        .as_uncompressed()?
        .convert()?;
    println!("[+] Клиент с читами");

    let (mut legit_stream, legit_protocol) = rx.recv().await.unwrap();
    let _legit_login_start: c2s::LoginStart = RawPacket::read(&mut legit_stream)
        .await?
        .as_uncompressed()?
        .convert()?;
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

    if cheat_protocol != 754 {
        error_handler(
            &mut cheat_stream,
            &mut legit_stream,
            "Для стабильности поддерживается только 1.16.5\nЕсли вы не можете выбрать версию в клиенте, то используйте ViaProxy".to_string(),
        )
        .await;
        return Ok(());
    }

    println!("Ник: {}", &cheat_login_start.name);

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

    let handshake = c2s::Handshake {
        protocol_version: VarInt(cheat_protocol),
        server_address: remote_dns,
        server_port: 25565,
        intent: VarInt(2),
    };

    handshake
        .as_uncompressed()?
        .to_raw_packet()?
        .write(&mut remote_stream)
        .await?;
    println!("[+] Handshake");

    cheat_login_start
        .as_uncompressed()?
        .to_raw_packet()?
        .write(&mut remote_stream)
        .await?;
    println!("[+] Login start");

    let mut threshold = None;

    loop {
        let packet = RawPacket::read(&mut remote_stream)
            .await?
            .try_uncompress(threshold)?
            .unwrap();

        match packet.packet_id.0 {
            0 => {
                error_handler(
                    &mut cheat_stream,
                    &mut legit_stream,
                    packet.convert::<s2c::LoginDisconnect>()?.reason,
                )
                .await;
                return Err(anyhow!("Disconnected"));
            }
            1 => {
                error_handler(
                    &mut cheat_stream,
                    &mut legit_stream,
                    "Лицензионный сервер пока не поддерживается".to_string(),
                )
                .await;
                return Err(anyhow!("Licensed"));
            }
            2 => {
                let packet = match threshold {
                    Some(t) => packet.compress(t as usize)?.to_raw_packet(),
                    None => packet.to_raw_packet()?,
                };
                packet.write(&mut cheat_stream).await?;
                packet.write(&mut legit_stream).await?;
                println!("[+] Login success");
                break;
            }
            3 => {
                let compression: s2c::SetCompression = packet.convert()?;
                threshold = Some(compression.threshold.0);

                let packet = packet.to_raw_packet()?;
                packet.write(&mut cheat_stream).await?;
                packet.write(&mut legit_stream).await?;
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

    let (event_tx, event_rx) = mpsc::channel(100);
    let (cheat_tx, cheat_rx) = mpsc::channel(100);
    let (legit_tx, legit_rx) = mpsc::channel(100);
    let (remote_tx, remote_rx) = mpsc::channel(100);

    let controller = Controller::new(
        ClientId::C,
        cheat_tx,
        legit_tx,
        remote_tx,
        event_rx,
        threshold,
    );

    tokio::spawn(run_client(
        cheat_read,
        cheat_write,
        ClientId::C,
        event_tx.clone(),
        cheat_rx,
    ));

    tokio::spawn(run_client(
        legit_read,
        legit_write,
        ClientId::L,
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
    let disconnect = s2c::LoginDisconnect {
        reason: json!({"text": message}).to_string(),
    };

    disconnect
        .as_uncompressed()
        .unwrap()
        .to_raw_packet()
        .unwrap()
        .write(cheat_stream)
        .await
        .unwrap();

    disconnect
        .as_uncompressed()
        .unwrap()
        .to_raw_packet()
        .unwrap()
        .write(legit_stream)
        .await
        .unwrap();
}
