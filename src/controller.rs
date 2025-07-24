use minecraft_protocol::{
    packet::{RawPacket, UncompressedPacket},
    varint::VarInt,
};
use tokio::{
    net::tcp::{OwnedReadHalf, OwnedWriteHalf},
    sync::mpsc::{Receiver, Sender},
};

use crate::packets::p767::{c2s, s2c};

// Идентификаторы клиентов
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientId {
    C,
    L,
}

// События для регулятора
#[derive(Debug)]
pub enum Event {
    ClientData(ClientId, RawPacket),
    ClientDisconnected(ClientId),
    ServerData(RawPacket),
    // SwitchActive,
}

// Регулятор
pub struct Controller {
    active_client: ClientId,
    cheat_tx: Sender<RawPacket>,
    legit_tx: Sender<RawPacket>,
    remote_tx: Sender<RawPacket>,
    event_rx: Receiver<Event>,
    threshold: Option<i32>,
    both_active: bool,
    position: s2c::Position,
    last_action: i16,
    need_sync: bool,
}

impl Controller {
    pub fn new(
        active_client: ClientId,
        cheat_tx: Sender<RawPacket>,
        legit_tx: Sender<RawPacket>,
        remote_tx: Sender<RawPacket>,
        event_rx: Receiver<Event>,
        threshold: Option<i32>,
    ) -> Self {
        Self {
            active_client,
            cheat_tx,
            legit_tx,
            remote_tx,
            event_rx,
            threshold,
            both_active: true,
            position: s2c::Position {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                yaw: 0.0,
                pitch: 0.0,
                flags: 0,
                teleportid: VarInt(0),
            },
            last_action: 0,
            need_sync: false,
        }
    }
    pub async fn run(mut self) {
        while let Some(event) = self.event_rx.recv().await {
            match event {
                Event::ClientData(client_id, packet) => {
                    if client_id == self.active_client {
                        if self.both_active {
                            if let Ok(Some(packet)) = packet.try_uncompress(self.threshold) {
                                match packet.packet_id.0 {
                                    0x12 | 0x13 | 0x14 => {
                                        let _ = self.update_position(&packet);
                                        // Уведомляем пассивного клиента
                                        let passive = match self.active_client {
                                            ClientId::C => ClientId::L,
                                            ClientId::L => ClientId::C,
                                        };

                                        let notice = match self.threshold {
                                            Some(t) => self
                                                .position
                                                .as_uncompressed()
                                                .unwrap()
                                                .compress(t as usize)
                                                .unwrap()
                                                .to_raw_packet(),
                                            None => self
                                                .position
                                                .as_uncompressed()
                                                .unwrap()
                                                .to_raw_packet()
                                                .unwrap(),
                                        };
                                        match passive {
                                            ClientId::C => self.cheat_tx.send(notice).await.ok(),
                                            ClientId::L => self.legit_tx.send(notice).await.ok(),
                                        };
                                    }

                                    _ => {}
                                }
                            }
                        }

                        if let Ok(Some(packet)) = packet.try_uncompress(self.threshold) {
                            if packet.packet_id.0 == 0x07 {
                                if let Ok(t) = packet.convert::<c2s::Transaction>() {
                                    if t.action < -500 {
                                        if self.need_sync {
                                            println!(
                                                "Синхронизация: {} -> {}",
                                                t.action, self.last_action
                                            );
                                            if self.last_action <= t.action {
                                                continue;
                                            } else if self.last_action == t.action + 1 {
                                                self.need_sync = false;
                                                self.last_action = t.action;
                                            } else {
                                                for i in (t.action + 1..=self.last_action - 1).rev()
                                                {
                                                    println!("Синхронизация: Отправка {}", i);
                                                    let mut new_transaction = t.clone();
                                                    new_transaction.action = i;
                                                    let new_transaction = match self.threshold {
                                                        Some(t) => new_transaction
                                                            .as_uncompressed()
                                                            .unwrap()
                                                            .compress(t as usize)
                                                            .unwrap()
                                                            .to_raw_packet(),
                                                        None => new_transaction
                                                            .as_uncompressed()
                                                            .unwrap()
                                                            .to_raw_packet()
                                                            .unwrap(),
                                                    };
                                                    self.remote_tx
                                                        .send(new_transaction)
                                                        .await
                                                        .unwrap();
                                                }
                                                self.need_sync = false;
                                                self.last_action = t.action;
                                            }
                                        } else {
                                            self.last_action = t.action;
                                        }
                                    }
                                }
                            }
                        }
                        // Перенаправляем активного клиента на сервер
                        if let Err(e) = self.remote_tx.send(packet).await {
                            eprintln!("Server send error: {}", e);
                        }
                    }
                }
                Event::ClientDisconnected(client_id) => {
                    if !self.both_active {
                        println!("Оба клиента отключились");
                        return;
                    }
                    self.both_active = false;
                    if self.active_client == client_id {
                        // Автоматическое переключение при отключении активного
                        self.active_client = match client_id {
                            ClientId::C => ClientId::L,
                            ClientId::L => ClientId::C,
                        };
                        println!("Переключился на {:?}", self.active_client);
                        self.need_sync = true;
                    }
                }
                Event::ServerData(packet) => {
                    // Рассылаем всем клиентам
                    self.cheat_tx.send(packet.clone()).await.ok();
                    self.legit_tx.send(packet).await.ok();
                } // Event::SwitchActive => {
                  //     if self.can_switch {
                  //         self.active_client = match self.active_client {
                  //             ClientId::C => ClientId::L,
                  //             ClientId::L => ClientId::C,
                  //         };
                  //         println!(
                  //             "Manually switched active client to {:?}",
                  //             self.active_client
                  //         );
                  //     }
                  // }
            }
        }
    }

    fn update_position(&mut self, packet: &UncompressedPacket) -> anyhow::Result<()> {
        match packet.packet_id.0 {
            0x12 => {
                let pos: c2s::Position = packet.convert()?;
                self.position.x = pos.x;
                self.position.y = pos.y;
                self.position.z = pos.z;
            }
            0x13 => {
                let pos: c2s::PositionLook = packet.convert()?;
                self.position.x = pos.x;
                self.position.y = pos.y;
                self.position.z = pos.z;
                self.position.yaw = pos.yaw;
                self.position.pitch = pos.pitch;
            }
            0x14 => {
                let pos: c2s::Look = packet.convert()?;
                self.position.yaw = pos.yaw;
                self.position.pitch = pos.pitch;
            }

            _ => {}
        };

        Ok(())
    }
}

// Запуск клиентского обработчика
pub async fn run_client(
    read_half: OwnedReadHalf,
    write_half: OwnedWriteHalf,
    client_id: ClientId,
    event_tx: Sender<Event>,
    mut packet_rx: Receiver<RawPacket>,
) {
    let (mut client_read, mut client_write) = (read_half, write_half);
    let _ = tokio::join!(
        async move {
            loop {
                match RawPacket::read(&mut client_read).await {
                    Ok(packet) => {
                        if client_id == ClientId::C {}
                        if event_tx
                            .send(Event::ClientData(client_id, packet))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(_) => {
                        event_tx
                            .send(Event::ClientDisconnected(client_id))
                            .await
                            .ok();
                        break;
                    }
                }
            }
        },
        async move {
            while let Some(packet) = packet_rx.recv().await {
                if packet.write(&mut client_write).await.is_err() {
                    break;
                }
            }
        }
    );
}

// Запуск серверного обработчика
pub async fn run_server(
    read_half: tokio::net::tcp::OwnedReadHalf,
    write_half: tokio::net::tcp::OwnedWriteHalf,
    event_tx: Sender<Event>,
    mut packet_rx: Receiver<RawPacket>,
) {
    let (mut server_read, mut server_write) = (read_half, write_half);
    let _ = tokio::join!(
        async move {
            loop {
                match RawPacket::read(&mut server_read).await {
                    Ok(packet) => {
                        if event_tx.send(Event::ServerData(packet)).await.is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        },
        async move {
            while let Some(packet) = packet_rx.recv().await {
                if packet.write(&mut server_write).await.is_err() {
                    break;
                }
            }
        }
    );
}
