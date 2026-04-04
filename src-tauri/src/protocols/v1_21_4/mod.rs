pub mod packets;

use mc_protocol::{
    packet::{RawPacket, UncompressedPacket},
    varint::VarInt,
};

use super::VersionProtocol;
use crate::{
    controller::ClientId,
    logger::Logger,
    protocols::{ClientBoundEvent, ClientDisconnectEvent, ServerBoundEvent},
};
use packets::{c2s, s2c};

const TELEPORT_ID: i32 = 1000;

#[derive(PartialEq, Debug)]
pub enum State {
    Login,
    Configuration,
    Game,
}

pub struct VersionData {
    pub active_state_in: State,
    pub active_state_out: State,
    pub inactive_state_in: State,
    pub inactive_state_out: State,
    pub position: s2c::game::Position,
    pub pings: Vec<PingSync>,
    pub threshold: Option<i32>,
    log: Logger,
}

impl VersionData {
    pub fn new(log: Logger) -> Self {
        Self {
            active_state_in: State::Login,
            active_state_out: State::Login,
            inactive_state_in: State::Login,
            inactive_state_out: State::Login,
            position: s2c::game::Position {
                id: VarInt(TELEPORT_ID),
                x: 0.0,
                y: 0.0,
                z: 0.0,
                delta_x: 0.0,
                delta_y: 0.0,
                delta_z: 0.0,
                yaw: 0.0,
                pitch: 0.0,
                relative_flags: 0,
            },
            pings: vec![],
            threshold: None,
            log,
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
        if is_active {
            match self.active_state_out {
                State::Login => self.handle_c2s_login(packet, is_active),
                State::Configuration => self.handle_c2s_configuration(packet, is_active),
                State::Game => self.handle_c2s_game(packet, client_id, is_active, both_active),
            }
            .unwrap_or_default()
        } else {
            match self.inactive_state_out {
                State::Login => self.handle_c2s_login(packet, is_active),
                State::Configuration => self.handle_c2s_configuration(packet, is_active),
                State::Game => self.handle_c2s_game(packet, client_id, is_active, both_active),
            }
            .unwrap_or_default()
        }
    }

    fn handle_s2c(&mut self, packet: &RawPacket, both_active: bool) -> Option<ClientBoundEvent> {
        match self.active_state_in {
            State::Login => Ok(None),
            State::Configuration => self.handle_s2c_configuration(packet),
            State::Game => self.handle_s2c_game(packet, both_active),
        }
        .unwrap_or_default()
    }

    fn update_threshold(&mut self, threshould: Option<i32>) {
        self.threshold = threshould;
    }

    fn handle_client_disconnect(&mut self, new_active: ClientId) -> Option<ClientDisconnectEvent> {
        let mut packets = vec![];

        let log = self.log.clone();
        self.pings.retain(|t| {
            if t.is_sent(new_active) {
                log.info(format!("Синхронизация: Отправка: {}", t.id));
                let tx = c2s::game::Pong { id: t.id };
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
}

impl VersionData {
    fn handle_s2c_configuration(
        &mut self,
        packet: &RawPacket,
    ) -> anyhow::Result<Option<ClientBoundEvent>> {
        let packet = packet.uncompress(self.threshold)?;

        match packet.packet_id {
            s2c::configuration::FinishConfiguration::PACKET_ID => {
                self.active_state_in = State::Game;
                self.inactive_state_in = State::Game;
            }
            _ => {}
        }
        Ok(None)
    }
    fn handle_s2c_game(
        &mut self,
        packet: &RawPacket,
        both_active: bool,
    ) -> anyhow::Result<Option<ClientBoundEvent>> {
        let packet = packet.uncompress(self.threshold)?;

        match packet.packet_id {
            s2c::game::StartConfiguration::PACKET_ID => {
                self.active_state_in = State::Configuration;
                self.inactive_state_in = State::Configuration;
            }
            s2c::game::Ping::PACKET_ID => {
                if both_active {
                    let ping: s2c::game::Ping = packet.deserialize_payload()?;
                    self.pings.push(PingSync::new(ping.id));
                }
            }
            _ => {}
        }

        Ok(None)
    }

    fn handle_c2s_login(
        &mut self,
        packet: &RawPacket,
        is_active: bool,
    ) -> anyhow::Result<Option<ServerBoundEvent>> {
        let packet = packet.uncompress(self.threshold)?;

        match packet.packet_id {
            c2s::login::LoginAcknowledged::PACKET_ID => {
                if is_active {
                    self.active_state_in = State::Configuration;
                    self.active_state_out = State::Configuration;
                } else {
                    self.inactive_state_in = State::Configuration;
                    self.inactive_state_out = State::Configuration;
                }
            }
            _ => {}
        }

        Ok(None)
    }

    fn handle_c2s_configuration(
        &mut self,
        packet: &RawPacket,
        is_active: bool,
    ) -> anyhow::Result<Option<ServerBoundEvent>> {
        let packet = packet.uncompress(self.threshold)?;

        match packet.packet_id {
            c2s::configuration::FinishConfiguration::PACKET_ID => {
                if is_active {
                    self.active_state_out = State::Game;
                } else {
                    self.inactive_state_out = State::Game;
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
            c2s::game::Pong::PACKET_ID => {
                let packet: c2s::game::Pong = packet.deserialize_payload()?;

                if both_active {
                    if let Some(i) = self
                        .pings
                        .iter_mut()
                        .position(|s| s.id == packet.id && s.sent(client_id))
                    {
                        self.pings.remove(i);
                    }
                } else {
                    if let Some(head) = self.pings.get(0) {
                        if head.is_sent(client_id.opposite()) {
                            self.log
                                .info(format!("Синхронизация: Пропуск: {}", head.id));
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

                    if self.inactive_state_in == State::Game {
                        return Ok(Some(ServerBoundEvent::SendToInactive(
                            UncompressedPacket::from_packet(&s2c::game::ContainerClose {
                                container_id: container_close.container_id,
                            })?
                            .to_raw_packet_compressed(self.threshold)?,
                        )));
                    }
                }
            }
            _ => {}
        };

        Ok(None)
    }

    fn send_position_to_inactive(&self) -> anyhow::Result<Option<ServerBoundEvent>> {
        if self.inactive_state_in == State::Game {
            Ok(Some(ServerBoundEvent::SendToInactive(
                UncompressedPacket::from_packet(&self.position)?
                    .to_raw_packet_compressed(self.threshold)?,
            )))
        } else {
            Ok(None)
        }
    }
}

#[derive(Debug)]
pub struct PingSync {
    pub id: i32,
    primary_sent: bool,
    secondary_sent: bool,
}

impl PingSync {
    pub fn new(id: i32) -> Self {
        Self {
            id,
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
