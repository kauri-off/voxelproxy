pub mod packets;

use std::collections::VecDeque;

use mc_protocol::{
    packet::{RawPacket, UncompressedPacket},
    varint::VarInt,
};

use super::{PacketIdReader, VersionProtocol, decode};
use crate::{
    config,
    events::{ClientId, PerClient},
    protocols::{ClientBoundEvent, ClientDisconnectEvent, ServerBoundEvent},
};
use packets::{c2s, s2c};

/// Vanilla servers never issue negative teleport ids.
const TELEPORT_ID: i32 = -1;

pub struct VersionData {
    pub position: s2c::game::Position,
    /// Teleports each client got and has not acked yet, in the order it got them.
    pub unacked: PerClient<VecDeque<Teleport>>,
    pub pings: Vec<PingSync>,
    pub teleports: Vec<TeleportSync>,
    pub keep_alives: Vec<KeepAliveSync>,
    /// Counts tracked pings, teleports and keep-alives in server stream order.
    pub seq: u64,
    pub threshold: Option<i32>,
    pub ids: PacketIdReader,
}

impl Default for VersionData {
    fn default() -> Self {
        Self::new()
    }
}

impl VersionData {
    pub fn new() -> Self {
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
            unacked: Default::default(),
            pings: vec![],
            teleports: vec![],
            keep_alives: vec![],
            seq: 0,
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
        both_online: bool,
    ) -> Option<ServerBoundEvent> {
        self.handle_c2s_game(packet, client_id, is_active, both_online)
            .unwrap_or_default()
    }

    fn handle_s2c(&mut self, packet: &RawPacket, both_online: bool) -> Option<ClientBoundEvent> {
        self.handle_s2c_game(packet, both_online)
            .unwrap_or_default()
    }

    fn update_threshold(&mut self, threshould: Option<i32>) {
        self.threshold = threshould;
    }

    fn handle_client_disconnect(&mut self, new_active: ClientId) -> Option<ClientDisconnectEvent> {
        let mut replies = vec![];

        self.pings.retain(|t| {
            if !t.answered(new_active) {
                return true;
            }
            if !t.sent {
                let tx = c2s::game::Ack {
                    container_id: t.container_id,
                    uid: t.uid,
                    accepted: true,
                };
                replies.push((
                    t.seq,
                    UncompressedPacket::from_packet(&tx)
                        .unwrap()
                        .to_raw_packet_compressed(self.threshold)
                        .unwrap(),
                ));
            }
            false
        });
        self.teleports.retain(|t| {
            if !t.answered(new_active) {
                return true;
            }
            if !t.sent {
                let tx = c2s::game::AcceptTeleportation { id: VarInt(t.id) };
                replies.push((
                    t.seq,
                    UncompressedPacket::from_packet(&tx)
                        .unwrap()
                        .to_raw_packet_compressed(self.threshold)
                        .unwrap(),
                ));
            }
            false
        });

        self.keep_alives.retain(|t| {
            if !t.answered(new_active) {
                return true;
            }
            if !t.sent {
                let tx = c2s::game::KeepAlive { id: t.id };
                replies.push((
                    t.seq,
                    UncompressedPacket::from_packet(&tx)
                        .unwrap()
                        .to_raw_packet_compressed(self.threshold)
                        .unwrap(),
                ));
            }
            false
        });

        // Vanilla clients answer pings and teleports in the order the server sent them.
        replies.sort_by_key(|(seq, _)| *seq);
        let packets: Vec<_> = replies.into_iter().map(|(_, packet)| packet).collect();

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
        both_online: bool,
    ) -> anyhow::Result<Option<ClientBoundEvent>> {
        let id = self.ids.read(packet, self.threshold)?;

        match id {
            s2c::game::ContainerAck::PACKET_ID if both_online => {
                let ping: s2c::game::ContainerAck = decode(packet, self.threshold)?;
                self.seq += 1;
                self.pings
                    .push(PingSync::new(ping.container_id, ping.uid, self.seq));
            }
            s2c::game::KeepAlive::PACKET_ID if both_online => {
                let keep_alive: s2c::game::KeepAlive = decode(packet, self.threshold)?;
                self.seq += 1;
                self.keep_alives
                    .push(KeepAliveSync::new(keep_alive.id, self.seq));
            }
            s2c::game::Position::PACKET_ID if both_online => {
                let teleport: s2c::game::Position = decode(packet, self.threshold)?;
                self.seq += 1;
                self.teleports
                    .push(TeleportSync::new(teleport.id.0, self.seq));
                for unacked in &mut self.unacked {
                    unacked.push_back(Teleport::Server {
                        id: teleport.id.0,
                        seq: self.seq,
                    });
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
        both_online: bool,
    ) -> anyhow::Result<Option<ServerBoundEvent>> {
        let id = self.ids.read(packet, self.threshold)?;

        match id {
            c2s::game::AcceptTeleportation::PACKET_ID => {
                let teleport: c2s::game::AcceptTeleportation = decode(packet, self.threshold)?;
                let unacked = &mut self.unacked[client_id];
                // Vanilla clients ack teleports in the order they got them.
                let acked = unacked
                    .iter()
                    .position(|t| t.id() == teleport.id.0)
                    .and_then(|i| unacked.remove(i));
                if acked == Some(Teleport::Own) {
                    return Ok(Some(ServerBoundEvent::SkipRelay));
                }
                if let Some(Teleport::Server { seq, .. }) = acked
                    && let Some(i) = self.teleports.iter().position(|t| t.seq == seq)
                {
                    let duplicate = self.teleports[i].answer(client_id, is_active);
                    if !both_online || self.teleports[i].answered_by_both() {
                        self.teleports.remove(i);
                    }
                    if duplicate {
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
                    return self.send_position_to_inactive(client_id, both_online);
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

                    return self.send_position_to_inactive(client_id, both_online);
                }
            }
            c2s::game::Rot::PACKET_ID => {
                if is_active {
                    let rot: c2s::game::Rot = decode(packet, self.threshold)?;

                    self.position.yaw = rot.yaw;
                    self.position.pitch = rot.pitch;
                    return self.send_position_to_inactive(client_id, both_online);
                }
            }
            c2s::game::Ack::PACKET_ID => {
                let packet: c2s::game::Ack = decode(packet, self.threshold)?;

                if let Some(i) = self.pings.iter().position(|s| {
                    s.uid == packet.uid
                        && s.container_id == packet.container_id
                        && !s.answered(client_id)
                }) {
                    let duplicate = self.pings[i].answer(client_id, is_active);
                    if !both_online || self.pings[i].answered_by_both() {
                        self.pings.remove(i);
                    }
                    if duplicate {
                        return Ok(Some(ServerBoundEvent::SkipRelay));
                    }
                }
            }
            c2s::game::KeepAlive::PACKET_ID => {
                let packet: c2s::game::KeepAlive = decode(packet, self.threshold)?;

                if let Some(i) = self
                    .keep_alives
                    .iter()
                    .position(|s| s.id == packet.id && !s.answered(client_id))
                {
                    let duplicate = self.keep_alives[i].answer(client_id, is_active);
                    if !both_online || self.keep_alives[i].answered_by_both() {
                        self.keep_alives.remove(i);
                    }
                    if duplicate {
                        return Ok(Some(ServerBoundEvent::SkipRelay));
                    }
                }
            }
            c2s::game::ContainerClose::PACKET_ID => {
                if is_active {
                    let container_close: c2s::game::ContainerClose =
                        decode(packet, self.threshold)?;

                    return Ok(Some(ServerBoundEvent::SendToInactive(
                        UncompressedPacket::from_packet(&s2c::game::ContainerClose {
                            container_id: container_close.container_id,
                        })?
                        .to_raw_packet_compressed(self.threshold)?,
                    )));
                }
            }
            c2s::game::Chat::PACKET_ID => {
                if is_active && let Ok(chat) = decode::<c2s::game::Chat>(packet, self.threshold) {
                    config::report_chat(chat.message, false);
                }
            }
            _ => {}
        };

        Ok(None)
    }

    fn send_position_to_inactive(
        &mut self,
        active: ClientId,
        both_online: bool,
    ) -> anyhow::Result<Option<ServerBoundEvent>> {
        if !both_online {
            return Ok(None);
        }
        let packet = UncompressedPacket::from_packet(&self.position)?
            .to_raw_packet_compressed(self.threshold)?;
        self.unacked[active.opposite()].push_back(Teleport::Own);
        Ok(Some(ServerBoundEvent::SendToInactive(packet)))
    }
}

/// A teleport a client got: one of ours or the server's.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Teleport {
    Own,
    Server { id: i32, seq: u64 },
}

impl Teleport {
    fn id(&self) -> i32 {
        match self {
            Self::Own => TELEPORT_ID,
            Self::Server { id, .. } => *id,
        }
    }
}

#[derive(Debug)]
pub struct PingSync {
    pub container_id: i8,
    pub uid: i16,
    pub seq: u64,
    answered: PerClient<bool>,
    /// Whether a reply already reached the server.
    sent: bool,
}

impl PingSync {
    pub fn new(container_id: i8, uid: i16, seq: u64) -> Self {
        Self {
            container_id,
            uid,
            seq,
            answered: PerClient::splat(false),
            sent: false,
        }
    }

    pub fn answered(&self, client: ClientId) -> bool {
        self.answered[client]
    }

    pub fn answered_by_both(&self) -> bool {
        self.answered == PerClient::splat(true)
    }

    /// Records a reply and returns whether the server already got one.
    pub fn answer(&mut self, client: ClientId, is_active: bool) -> bool {
        self.answered[client] = true;
        let duplicate = is_active && self.sent;
        self.sent |= is_active;
        duplicate
    }
}

#[derive(Debug)]
pub struct TeleportSync {
    pub id: i32,
    pub seq: u64,
    answered: PerClient<bool>,
    /// Whether an ack already reached the server.
    sent: bool,
}

impl TeleportSync {
    pub fn new(id: i32, seq: u64) -> Self {
        Self {
            id,
            seq,
            answered: PerClient::splat(false),
            sent: false,
        }
    }

    pub fn answered(&self, client: ClientId) -> bool {
        self.answered[client]
    }

    pub fn answered_by_both(&self) -> bool {
        self.answered == PerClient::splat(true)
    }

    /// Records an ack and returns whether the server already got one.
    pub fn answer(&mut self, client: ClientId, is_active: bool) -> bool {
        self.answered[client] = true;
        let duplicate = is_active && self.sent;
        self.sent |= is_active;
        duplicate
    }
}

#[derive(Debug)]
pub struct KeepAliveSync {
    pub id: i64,
    pub seq: u64,
    answered: PerClient<bool>,
    /// Whether an answer already reached the server.
    sent: bool,
}

impl KeepAliveSync {
    pub fn new(id: i64, seq: u64) -> Self {
        Self {
            id,
            seq,
            answered: PerClient::splat(false),
            sent: false,
        }
    }

    pub fn answered(&self, client: ClientId) -> bool {
        self.answered[client]
    }

    pub fn answered_by_both(&self) -> bool {
        self.answered == PerClient::splat(true)
    }

    /// Records an answer and returns whether the server already got one.
    pub fn answer(&mut self, client: ClientId, is_active: bool) -> bool {
        self.answered[client] = true;
        let duplicate = is_active && self.sent;
        self.sent |= is_active;
        duplicate
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controller::tests::{C2s, Phase, S2c, Spot, Wire, pack, unpack};

    impl Wire for VersionData {
        const CONFIGURATION: bool = false;

        fn write_c2s(_: Phase, packet: C2s) -> UncompressedPacket {
            match packet {
                C2s::Teleport(id, _) => pack(&c2s::game::AcceptTeleportation { id: VarInt(id) }),
                C2s::Pos(x, y, z) => pack(&c2s::game::Pos {
                    x,
                    y,
                    z,
                    on_ground: false,
                }),
                C2s::Rot(yaw, pitch) => pack(&c2s::game::Rot {
                    yaw,
                    pitch,
                    on_ground: false,
                }),
                C2s::PosRot(at) => pack(&c2s::game::PosRot {
                    x: at.x,
                    y: at.y,
                    z: at.z,
                    yaw: at.yaw,
                    pitch: at.pitch,
                    on_ground: false,
                }),
                C2s::Pong(id) => pack(&c2s::game::Ack {
                    container_id: 0,
                    uid: id as i16,
                    accepted: true,
                }),
                C2s::KeepAlive(id) => pack(&c2s::game::KeepAlive { id }),
                C2s::ContainerClose(id) => pack(&c2s::game::ContainerClose {
                    container_id: id as u8,
                }),
                C2s::Raw(id) => UncompressedPacket::new(id, vec![]),
                C2s::LoginAck
                | C2s::FinishConfiguration
                | C2s::ConfigurationAck
                | C2s::KnownPacks
                | C2s::AcceptCodeOfConduct
                | C2s::ResourcePack(..)
                | C2s::ChunkBatchReceived(_) => {
                    panic!("{packet:?} does not exist in 1.16.5")
                }
            }
        }

        fn write_s2c(_: Phase, packet: S2c) -> UncompressedPacket {
            match packet {
                S2c::Position(id, at) => pack(&s2c::game::Position {
                    x: at.x,
                    y: at.y,
                    z: at.z,
                    yaw: at.yaw,
                    pitch: at.pitch,
                    relative_flags: 0,
                    id: VarInt(id),
                }),
                // Only a rejected click makes the client answer, see `handleContainerAck`.
                S2c::Ping(id) => pack(&s2c::game::ContainerAck {
                    container_id: 0,
                    uid: id as i16,
                    accepted: false,
                }),
                S2c::KeepAlive(id) => pack(&s2c::game::KeepAlive { id }),
                S2c::ContainerClose(id) => pack(&s2c::game::ContainerClose {
                    container_id: id as u8,
                }),
                S2c::Raw(id, len) => UncompressedPacket::new(id, vec![0; len]),
                S2c::StartConfiguration
                | S2c::FinishConfiguration
                | S2c::SelectKnownPacks
                | S2c::CodeOfConduct
                | S2c::ResourcePackPush(_)
                | S2c::ChunkBatchFinished(_) => {
                    panic!("{packet:?} does not exist in 1.16.5")
                }
            }
        }

        fn read_c2s(_: Phase, packet: &UncompressedPacket) -> Option<C2s> {
            use c2s::game;
            Some(match packet.packet_id {
                game::AcceptTeleportation::PACKET_ID => C2s::Teleport(
                    unpack::<game::AcceptTeleportation>(packet).id.0,
                    Spot::default(),
                ),
                game::Pos::PACKET_ID => {
                    let p: game::Pos = unpack(packet);
                    C2s::Pos(p.x, p.y, p.z)
                }
                game::Rot::PACKET_ID => {
                    let p: game::Rot = unpack(packet);
                    C2s::Rot(p.yaw, p.pitch)
                }
                game::PosRot::PACKET_ID => {
                    let p: game::PosRot = unpack(packet);
                    C2s::PosRot(Spot {
                        x: p.x,
                        y: p.y,
                        z: p.z,
                        yaw: p.yaw,
                        pitch: p.pitch,
                    })
                }
                game::Ack::PACKET_ID => {
                    let p: game::Ack = unpack(packet);
                    assert_eq!((p.container_id, p.accepted), (0, true));
                    C2s::Pong(p.uid.into())
                }
                game::KeepAlive::PACKET_ID => C2s::KeepAlive(unpack::<game::KeepAlive>(packet).id),
                game::ContainerClose::PACKET_ID => {
                    C2s::ContainerClose(unpack::<game::ContainerClose>(packet).container_id.into())
                }
                _ => return None,
            })
        }

        fn read_s2c(_: Phase, packet: &UncompressedPacket) -> Option<S2c> {
            use s2c::game;
            Some(match packet.packet_id {
                game::Position::PACKET_ID => {
                    let p: game::Position = unpack(packet);
                    assert_eq!(p.relative_flags, 0);
                    S2c::Position(
                        p.id.0,
                        Spot {
                            x: p.x,
                            y: p.y,
                            z: p.z,
                            yaw: p.yaw,
                            pitch: p.pitch,
                        },
                    )
                }
                game::ContainerAck::PACKET_ID => {
                    S2c::Ping(unpack::<game::ContainerAck>(packet).uid.into())
                }
                game::KeepAlive::PACKET_ID => S2c::KeepAlive(unpack::<game::KeepAlive>(packet).id),
                game::ContainerClose::PACKET_ID => {
                    S2c::ContainerClose(unpack::<game::ContainerClose>(packet).container_id.into())
                }
                _ => return None,
            })
        }
    }

    crate::controller::tests::controller_tests!();
}
