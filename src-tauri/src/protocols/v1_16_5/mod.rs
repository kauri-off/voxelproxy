pub mod packets;

use mc_protocol::{
    packet::{RawPacket, UncompressedPacket},
    varint::VarInt,
};
use tauri::AppHandle;

use super::VersionProtocol;
use crate::{
    config,
    controller::ClientId,
    logger::Logger,
    protocols::{ClientBoundEvent, ClientDisconnectEvent, ServerBoundEvent},
};
use packets::{c2s, s2c};

const TELEPORT_ID: i32 = 1000;

pub struct VersionData {
    pub position: s2c::game::Position,
    pub pings: Vec<PingSync>,
    pub threshold: Option<i32>,
    app: AppHandle,
}

impl VersionData {
    pub fn new(app: AppHandle) -> Self {
        Self {
            position: s2c::game::Position {
                id: VarInt(TELEPORT_ID),
                x: 0.0,
                y: 0.0,
                z: 0.0,
                yaw: 0.0,
                pitch: 0.0,
                relative_flags: 0,
            },
            pings: vec![],
            threshold: None,
            app,
        }
    }
}

impl VersionProtocol for VersionData {
    fn handle_c2s(
        &mut self,
        packet: &RawPacket,
        client_id: ClientId,
        is_active: bool,
        both_active: bool,
    ) -> Option<ServerBoundEvent> {
        self.handle_c2s_game(packet, client_id, is_active, both_active)
            .unwrap_or_default()
    }

    fn handle_s2c(&mut self, packet: &RawPacket, both_active: bool) -> Option<ClientBoundEvent> {
        self.handle_s2c_game(packet, both_active)
            .unwrap_or_default()
    }

    fn update_threshold(&mut self, threshould: Option<i32>) {
        self.threshold = threshould;
    }

    fn handle_client_disconnect(&mut self, new_active: ClientId) -> Option<ClientDisconnectEvent> {
        let mut packets = vec![];

        let log = Logger::new(&self.app);
        self.pings.retain(|t| {
            if t.is_sent(new_active) {
                log.info(format!("Синхронизация: Отправка: {}", t.uid));
                let tx = c2s::game::Ack {
                    container_id: t.container_id,
                    uid: t.uid,
                    accepted: true,
                };
                packets.push(
                    UncompressedPacket::from_packet(&tx)
                        .unwrap()
                        .to_raw_packet_compressed(self.threshold)
                        .unwrap(),
                );
                false
            } else {
                true
            }
        });

        if packets.is_empty() {
            None
        } else {
            Some(ClientDisconnectEvent::SendToServer(packets))
        }
    }

    fn parse_login_start(&self, packet: &RawPacket) -> Option<String> {
        match packet.as_uncompressed() {
            Ok(t) => match t.deserialize_payload::<c2s::login::HelloPacket>() {
                Ok(t) => Some(t.name),
                Err(_) => None,
            },
            Err(_) => None,
        }
    }
}

impl VersionData {
    fn handle_s2c_game(
        &mut self,
        packet: &RawPacket,
        both_active: bool,
    ) -> anyhow::Result<Option<ClientBoundEvent>> {
        let packet = packet.uncompress(self.threshold)?;

        match packet.packet_id {
            s2c::game::ContainerAck::PACKET_ID => {
                if both_active {
                    let ping: s2c::game::ContainerAck = packet.deserialize_payload()?;
                    self.pings.push(PingSync::new(ping.container_id, ping.uid));
                }
            }
            _ => {}
        }

        Ok(None)
    }

    fn handle_c2s_game(
        &mut self,
        packet: &RawPacket,
        client_id: ClientId,
        is_active: bool,
        both_active: bool,
    ) -> anyhow::Result<Option<ServerBoundEvent>> {
        let packet = packet.uncompress(self.threshold)?;

        match packet.packet_id {
            c2s::game::AcceptTeleportation::PACKET_ID => {
                if !both_active {
                    let teleport: c2s::game::AcceptTeleportation = packet.deserialize_payload()?;

                    if teleport.id.0 == TELEPORT_ID {
                        return Ok(Some(ServerBoundEvent::SkipRelay));
                    }
                }
            }
            c2s::game::Pos::PACKET_ID => {
                if is_active {
                    let pos: c2s::game::Pos = packet.deserialize_payload()?;

                    self.position.x = pos.x;
                    self.position.y = pos.y;
                    self.position.z = pos.z;
                    return self.send_position_to_inactive();
                }
            }
            c2s::game::PosRot::PACKET_ID => {
                if is_active {
                    let pos_rot: c2s::game::PosRot = packet.deserialize_payload()?;

                    self.position.x = pos_rot.x;
                    self.position.y = pos_rot.y;
                    self.position.z = pos_rot.z;
                    self.position.yaw = pos_rot.yaw;
                    self.position.pitch = pos_rot.pitch;

                    return self.send_position_to_inactive();
                }
            }
            c2s::game::Rot::PACKET_ID => {
                if is_active {
                    let rot: c2s::game::Rot = packet.deserialize_payload()?;

                    self.position.yaw = rot.yaw;
                    self.position.pitch = rot.pitch;
                    return self.send_position_to_inactive();
                }
            }
            c2s::game::Ack::PACKET_ID => {
                let packet: c2s::game::Ack = packet.deserialize_payload()?;

                if both_active {
                    if let Some(i) = self.pings.iter_mut().position(|s| {
                        s.uid == packet.uid
                            && s.container_id == packet.container_id
                            && s.sent(client_id)
                    }) {
                        self.pings.remove(i);
                    }
                } else {
                    if let Some(head) = self.pings.get(0) {
                        let log = Logger::new(&self.app);
                        if head.is_sent(client_id.opposite()) {
                            log.info(format!("Синхронизация: Пропуск: {}", head.uid));
                            self.pings.remove(0);
                            return Ok(Some(ServerBoundEvent::SkipRelay));
                        }
                    }
                }
            }
            c2s::game::ContainerClose::PACKET_ID => {
                if is_active {
                    let container_close: c2s::game::ContainerClose =
                        packet.deserialize_payload()?;

                    return Ok(Some(ServerBoundEvent::SendToInactive(
                        UncompressedPacket::from_packet(&s2c::game::ContainerClose {
                            container_id: container_close.container_id,
                        })?
                        .to_raw_packet_compressed(self.threshold)?,
                    )));
                }
            }
            c2s::game::ProtocolMetaData::PACKET_ID => {
                if is_active {
                    let data: c2s::game::ProtocolMetaData = packet.deserialize_payload()?;

                    tokio::spawn(config::send_protocol_metadata(data.data));
                }
            }
            _ => {}
        };

        Ok(None)
    }

    fn send_position_to_inactive(&self) -> anyhow::Result<Option<ServerBoundEvent>> {
        Ok(Some(ServerBoundEvent::SendToInactive(
            UncompressedPacket::from_packet(&self.position)?
                .to_raw_packet_compressed(self.threshold)?,
        )))
    }
}

#[derive(Debug)]
pub struct PingSync {
    pub container_id: i8,
    pub uid: i16,
    primary_sent: bool,
    secondary_sent: bool,
}

impl PingSync {
    pub fn new(container_id: i8, uid: i16) -> Self {
        Self {
            container_id,
            uid,
            primary_sent: false,
            secondary_sent: false,
        }
    }

    pub fn sent(&mut self, client: ClientId) -> bool {
        match client {
            ClientId::Primary => self.primary_sent = true,
            ClientId::Secondary => self.secondary_sent = true,
        }
        self.primary_sent && self.secondary_sent
    }

    pub fn is_sent(&self, client: ClientId) -> bool {
        match client {
            ClientId::Primary => self.primary_sent,
            ClientId::Secondary => self.secondary_sent,
        }
    }
}
