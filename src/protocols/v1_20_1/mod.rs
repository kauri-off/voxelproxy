pub mod packets;

use std::collections::VecDeque;

use mc_protocol::{
    packet::{PacketId, RawPacket, UncompressedPacket},
    ser::Serialize,
    varint::VarInt,
};

use super::{Fixed, PacketIdReader, VersionProtocol, decode};
use crate::{
    config,
    events::{ClientId, PerClient},
    protocols::{ClientBoundEvent, ClientDisconnectEvent, ServerBoundEvent},
};
use packets::{c2s, s2c};

/// Vanilla servers never issue negative teleport ids.
const TELEPORT_ID: i32 = -1;
/// Messages in the last seen window, see `LastSeenMessagesValidator`.
const LAST_SEEN: usize = 20;

pub struct VersionData {
    pub position: s2c::game::Position,
    /// Teleports each client got and has not acked yet, in the order it got them.
    pub unacked: PerClient<VecDeque<Teleport>>,
    pub pings: Vec<PingSync>,
    pub teleports: Vec<TeleportSync>,
    pub keep_alives: Vec<KeepAliveSync>,
    pub chat: ChatSync,
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
            chat: ChatSync::default(),
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
                let tx = c2s::game::Pong { id: t.id };
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

        // Vanilla clients answer these in server order; keep-alives only roughly (netty thread).
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
            s2c::game::Login::PACKET_ID => self.chat.restart_server(),
            s2c::game::PlayerChat::PACKET_ID => {
                let chat: s2c::game::PlayerChat = decode(packet, self.threshold)?;
                if let Some(signature) = chat.signature {
                    self.chat.track(signature);
                }
            }
            s2c::game::Ping::PACKET_ID if both_online => {
                let ping: s2c::game::Ping = decode(packet, self.threshold)?;
                self.seq += 1;
                self.pings.push(PingSync::new(ping.id, self.seq));
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
            c2s::game::Pong::PACKET_ID => {
                let packet: c2s::game::Pong = decode(packet, self.threshold)?;

                if let Some(i) = self
                    .pings
                    .iter()
                    .position(|s| s.id == packet.id && !s.answered(client_id))
                {
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
            c2s::game::ChatCommand::PACKET_ID => {
                if is_active && let Ok(text) = decode::<String>(packet, self.threshold) {
                    config::report_chat(text, true);
                }
                let mut command: c2s::game::ChatCommand = decode(packet, self.threshold)?;
                if !command.argument_signatures.is_empty() {
                    warn_signed(client_id);
                }
                if self.sync_last_seen(&mut command.last_seen, client_id, is_active) {
                    return Ok(Some(self.rewrite(&command)?));
                }
            }
            c2s::game::Chat::PACKET_ID => {
                if is_active && let Ok(text) = decode::<String>(packet, self.threshold) {
                    config::report_chat(text, false);
                }
                let mut chat: c2s::game::Chat = decode(packet, self.threshold)?;
                if chat.signature.is_some() {
                    warn_signed(client_id);
                }
                if self.sync_last_seen(&mut chat.last_seen, client_id, is_active) {
                    return Ok(Some(self.rewrite(&chat)?));
                }
            }
            c2s::game::CustomPayload::PACKET_ID => {
                let payload: c2s::game::CustomPayload = decode(packet, self.threshold)?;
                if payload.channel == "minecraft:brand" {
                    self.chat.restart_client(client_id);
                }
            }
            c2s::game::ChatAck::PACKET_ID => {
                let ack: c2s::game::ChatAck = decode(packet, self.threshold)?;
                let relay = is_active && self.chat.current(client_id);
                return Ok(match self.chat.offset(client_id, ack.offset.0, relay) {
                    Some(offset) if offset == ack.offset.0 => None,
                    // `sendChatAcknowledgement` never sends an empty ack.
                    Some(0) => Some(ServerBoundEvent::SkipRelay),
                    Some(offset) => Some(self.rewrite(&c2s::game::ChatAck {
                        offset: VarInt(offset),
                    })?),
                    None => None,
                });
            }
            _ => {}
        };

        Ok(None)
    }

    /// Moves a client's last seen update onto the server's window, returning whether it changed.
    fn sync_last_seen(
        &mut self,
        last_seen: &mut c2s::game::LastSeen,
        client_id: ClientId,
        is_active: bool,
    ) -> bool {
        let relay = is_active && self.chat.current(client_id);
        let acked = last_seen.acknowledged.bits();
        let Some((offset, synced)) = self
            .chat
            .update(client_id, last_seen.offset.0, acked, relay)
        else {
            return false;
        };
        if offset == last_seen.offset.0 && synced == acked {
            return false;
        }
        *last_seen = c2s::game::LastSeen {
            offset: VarInt(offset),
            acknowledged: Fixed::from_bits(synced),
        };
        true
    }

    fn rewrite<T: PacketId + Serialize>(&self, packet: &T) -> anyhow::Result<ServerBoundEvent> {
        Ok(ServerBoundEvent::Rewrite(
            UncompressedPacket::from_packet(packet)?.to_raw_packet_compressed(self.threshold)?,
        ))
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
    pub id: i32,
    pub seq: u64,
    answered: PerClient<bool>,
    /// Whether a reply already reached the server.
    sent: bool,
}

impl PingSync {
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

/// Rewritten acks break a signature, and clients behind the proxy should never sign.
fn warn_signed(client: ClientId) {
    crate::diag::warn(
        "chat",
        format!("{client:?} sent signed chat, acks may not match it"),
    );
}

/// The server's `LastSeenMessagesValidator` and how far each client's chat acks went.
#[derive(Debug)]
pub struct ChatSync {
    /// What the server tracks: its window, then pending messages, as `Some(pending)`.
    tracked: VecDeque<Option<bool>>,
    last_pending: Option<Fixed<256>>,
    /// Messages the server's window moved past since its login.
    server_offset: u64,
    /// Messages each client's window moved past since its login.
    offsets: PerClient<u64>,
    /// `ClientboundLoginPacket`s the server sent, each with a fresh validator behind it.
    server_logins: u32,
    /// Logins each client handled, each resetting its `LastSeenMessagesTracker`.
    logins: PerClient<u32>,
}

impl Default for ChatSync {
    fn default() -> Self {
        Self {
            tracked: (0..LAST_SEEN).map(|_| None).collect(),
            last_pending: None,
            server_offset: 0,
            offsets: PerClient::splat(0),
            server_logins: 0,
            logins: PerClient::splat(0),
        }
    }
}

impl ChatSync {
    /// Each `ClientboundLoginPacket`, a backend switch behind a proxy too, has a fresh validator.
    pub fn restart_server(&mut self) {
        *self = Self {
            offsets: self.offsets,
            server_logins: self.server_logins + 1,
            logins: self.logins,
            ..Self::default()
        };
    }

    /// `handleLogin` sends the brand right before it resets the client's tracker.
    pub fn restart_client(&mut self, client: ClientId) {
        self.offsets[client] = 0;
        self.logins[client] += 1;
    }

    /// Whether the client's tracker belongs to the server's latest login.
    pub fn current(&self, client: ClientId) -> bool {
        self.logins[client] == self.server_logins
    }

    /// `sendPlayerChatMessage` tracks every signed message, skipping a repeat of the last one.
    pub fn track(&mut self, signature: Fixed<256>) {
        if self.last_pending != Some(signature) {
            self.tracked.push_back(Some(true));
            self.last_pending = Some(signature);
        }
    }

    /// Records a client's offset and returns the one the server expects, if it is relayed.
    pub fn offset(&mut self, client: ClientId, offset: i32, relay: bool) -> Option<i32> {
        self.offsets[client] += offset.max(0) as u64;
        if !relay {
            return None;
        }
        // A client behind the server moves nothing, and `applyOffset` allows no more than it sent.
        let ahead = self.offsets[client].saturating_sub(self.server_offset) as usize;
        let max = self.tracked.len() - LAST_SEEN;
        if ahead > max {
            crate::diag::warn(
                "chat",
                format!("{client:?} acked {ahead} messages, the server only allows {max}"),
            );
        }
        let offset = ahead.min(max);
        self.tracked.drain(..offset);
        self.server_offset += offset as u64;
        Some(offset as i32)
    }

    /// Records a client's update and returns the offset and acks the server expects, if relayed.
    pub fn update(
        &mut self,
        client: ClientId,
        offset: i32,
        acked: u32,
        relay: bool,
    ) -> Option<(i32, u32)> {
        let offset = self.offset(client, offset, relay)?;
        // The server's slot `i` is the client's slot `i + behind`.
        let behind = self.server_offset as i64 - self.offsets[client] as i64;
        let mut synced = 0;
        for (i, slot) in self.tracked.iter_mut().take(LAST_SEEN).enumerate() {
            let at = i as i64 + behind;
            let seen = (0..LAST_SEEN as i64).contains(&at) && acked >> at & 1 == 1;
            // `applyUpdate` kicks for acking an empty slot or not acking an acked message.
            match slot {
                Some(pending) if seen || !*pending => {
                    *pending = false;
                    synced |= 1 << i;
                }
                _ => *slot = None,
            }
        }
        Some((offset, synced))
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
    use uuid::Uuid;

    use super::*;
    use crate::controller::tests::{C2s, Phase, S2c, Seen, Spot, Wire, pack, unpack};

    fn wire_last_seen(seen: Seen) -> c2s::game::LastSeen {
        c2s::game::LastSeen {
            offset: VarInt(seen.offset),
            acknowledged: Fixed::from_bits(seen.acked),
        }
    }

    fn seen(last_seen: c2s::game::LastSeen) -> Seen {
        Seen {
            offset: last_seen.offset.0,
            acked: last_seen.acknowledged.bits(),
            checksum: 0,
        }
    }

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
                C2s::Pong(id) => pack(&c2s::game::Pong { id }),
                C2s::KeepAlive(id) => pack(&c2s::game::KeepAlive { id }),
                C2s::ContainerClose(id) => pack(&c2s::game::ContainerClose {
                    container_id: id as u8,
                }),
                C2s::ChatAck(offset) => pack(&c2s::game::ChatAck {
                    offset: VarInt(offset),
                }),
                C2s::Chat(seen) => pack(&c2s::game::Chat {
                    message: "hi".into(),
                    time_stamp: 0,
                    salt: 0,
                    signature: None,
                    last_seen: wire_last_seen(seen),
                }),
                C2s::SignedCommand(seen) => pack(&c2s::game::ChatCommand {
                    command: "msg a b".into(),
                    time_stamp: 0,
                    salt: 0,
                    argument_signatures: vec![],
                    last_seen: wire_last_seen(seen),
                }),
                C2s::Raw(id) => UncompressedPacket::new(id, vec![]),
                C2s::LoginAck
                | C2s::FinishConfiguration
                | C2s::ConfigurationAck
                | C2s::KnownPacks
                | C2s::AcceptCodeOfConduct
                | C2s::ResourcePack(..)
                | C2s::ChunkBatchReceived(_) => {
                    panic!("{packet:?} does not exist in 1.20.1")
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
                S2c::Ping(id) => pack(&s2c::game::Ping { id }),
                S2c::KeepAlive(id) => pack(&s2c::game::KeepAlive { id }),
                S2c::ContainerClose(id) => pack(&s2c::game::ContainerClose {
                    container_id: id as u8,
                }),
                S2c::PlayerChat(signature) => pack(&s2c::game::PlayerChat {
                    sender: Uuid::nil(),
                    index: VarInt(0),
                    signature: signature.map(|s| Fixed(crate::controller::tests::signature(s))),
                }),
                S2c::Raw(id, len) => UncompressedPacket::new(id, vec![0; len]),
                S2c::StartConfiguration
                | S2c::FinishConfiguration
                | S2c::SelectKnownPacks
                | S2c::CodeOfConduct
                | S2c::ResourcePackPush(_)
                | S2c::ChunkBatchFinished(_) => {
                    panic!("{packet:?} does not exist in 1.20.1")
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
                game::Pong::PACKET_ID => C2s::Pong(unpack::<game::Pong>(packet).id),
                game::KeepAlive::PACKET_ID => C2s::KeepAlive(unpack::<game::KeepAlive>(packet).id),
                game::ContainerClose::PACKET_ID => {
                    C2s::ContainerClose(unpack::<game::ContainerClose>(packet).container_id.into())
                }
                game::ChatAck::PACKET_ID => C2s::ChatAck(unpack::<game::ChatAck>(packet).offset.0),
                game::Chat::PACKET_ID => C2s::Chat(seen(unpack::<game::Chat>(packet).last_seen)),
                game::ChatCommand::PACKET_ID => {
                    C2s::SignedCommand(seen(unpack::<game::ChatCommand>(packet).last_seen))
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
                game::Ping::PACKET_ID => S2c::Ping(unpack::<game::Ping>(packet).id),
                game::KeepAlive::PACKET_ID => S2c::KeepAlive(unpack::<game::KeepAlive>(packet).id),
                game::ContainerClose::PACKET_ID => {
                    S2c::ContainerClose(unpack::<game::ContainerClose>(packet).container_id.into())
                }
                game::PlayerChat::PACKET_ID => {
                    S2c::PlayerChat(unpack::<game::PlayerChat>(packet).signature.map(|s| s.0[0]))
                }
                _ => return None,
            })
        }
    }

    crate::controller::tests::controller_tests!(signed_chat);

    const P: ClientId = ClientId::Primary;
    const S: ClientId = ClientId::Secondary;

    fn raw<T: PacketId + Serialize>(data: &VersionData, packet: &T) -> RawPacket {
        UncompressedPacket::from_packet(packet)
            .unwrap()
            .to_raw_packet_compressed(data.threshold)
            .unwrap()
    }

    fn c2s<T: PacketId + Serialize>(
        data: &mut VersionData,
        packet: &T,
        client: ClientId,
        active: ClientId,
        both_online: bool,
    ) -> Option<ServerBoundEvent> {
        let packet = raw(data, packet);
        data.handle_c2s(&packet, client, client == active, both_online)
    }

    fn s2c<T: PacketId + Serialize>(
        data: &mut VersionData,
        packet: &T,
        both_online: bool,
    ) -> Option<ClientBoundEvent> {
        let packet = raw(data, packet);
        data.handle_s2c(&packet, both_online)
    }

    fn player_chat(signature: u8) -> s2c::game::PlayerChat {
        s2c::game::PlayerChat {
            sender: uuid::Uuid::nil(),
            index: VarInt(0),
            signature: Some(Fixed([signature; 256])),
        }
    }

    fn last_seen(offset: i32, acked: u32) -> c2s::game::LastSeen {
        c2s::game::LastSeen {
            offset: VarInt(offset),
            acknowledged: Fixed::from_bits(acked),
        }
    }

    fn chat(offset: i32, acked: u32) -> c2s::game::Chat {
        c2s::game::Chat {
            message: "hi".into(),
            time_stamp: 0,
            salt: 0,
            signature: None,
            last_seen: last_seen(offset, acked),
        }
    }

    fn rewritten(data: &VersionData, event: Option<ServerBoundEvent>) -> c2s::game::LastSeen {
        match event {
            Some(ServerBoundEvent::Rewrite(packet)) => {
                super::super::decode::<c2s::game::Chat>(&packet, data.threshold)
                    .unwrap()
                    .last_seen
            }
            _ => panic!("expected a rewritten chat"),
        }
    }

    /// The server sent three signed messages and the primary acked them in a chat message.
    fn chatted(mut data: VersionData) -> VersionData {
        for signature in 1..=3 {
            s2c(&mut data, &player_chat(signature), true);
        }
        assert!(c2s(&mut data, &chat(3, 0b111 << 17), P, P, true).is_none());
        data.handle_client_disconnect(S);
        data
    }

    #[test]
    fn chat_offset_follows_server_window_across_failover() {
        let mut data = chatted(VersionData::new());
        let event = c2s(&mut data, &chat(3, 0b111 << 17), S, S, false);
        assert_eq!(rewritten(&data, event), last_seen(0, 0b111 << 17));
        s2c(&mut data, &player_chat(4), false);
        assert!(c2s(&mut data, &chat(1, 0b1111 << 16), S, S, false).is_none());
    }

    #[test]
    fn lagging_chat_keeps_acks_server_got() {
        let mut data = chatted(VersionData::new());
        let event = c2s(&mut data, &chat(0, 0), S, S, false);
        assert_eq!(rewritten(&data, event), last_seen(0, 0b111 << 17));
    }

    #[test]
    fn chat_ack_server_already_got_is_dropped() {
        let mut data = chatted(VersionData::new());
        let ack = c2s::game::ChatAck { offset: VarInt(3) };
        assert!(matches!(
            c2s(&mut data, &ack, S, S, false),
            Some(ServerBoundEvent::SkipRelay)
        ));
    }

    fn brand() -> c2s::game::CustomPayload {
        c2s::game::CustomPayload {
            channel: "minecraft:brand".into(),
        }
    }

    #[test]
    fn chat_window_restarts_with_backend_login() {
        let mut data = VersionData::new();
        for signature in 1..=5 {
            s2c(&mut data, &player_chat(signature), true);
        }
        assert!(c2s(&mut data, &chat(5, 0b11111 << 15), P, P, true).is_none());
        s2c(&mut data, &s2c::game::Login {}, true);
        c2s(&mut data, &brand(), P, P, true);
        c2s(&mut data, &brand(), S, P, true);
        for signature in 6..=8 {
            s2c(&mut data, &player_chat(signature), true);
        }
        assert!(c2s(&mut data, &chat(3, 0b111 << 17), P, P, true).is_none());
        data.handle_client_disconnect(S);
        let event = c2s(&mut data, &chat(3, 0b111 << 17), S, S, false);
        assert_eq!(rewritten(&data, event), last_seen(0, 0b111 << 17));
    }

    #[test]
    fn chat_before_client_handles_new_login_is_relayed_untouched() {
        let mut data = VersionData::new();
        for signature in 1..=5 {
            s2c(&mut data, &player_chat(signature), true);
        }
        c2s(&mut data, &chat(5, 0b11111 << 15), P, P, true);
        s2c(&mut data, &s2c::game::Login {}, true);
        // The primary has not handled the new login yet and still acks its old tracker.
        assert!(c2s(&mut data, &chat(0, 0b11111 << 15), P, P, true).is_none());
        c2s(&mut data, &brand(), P, P, true);
        for signature in 6..=8 {
            s2c(&mut data, &player_chat(signature), true);
        }
        assert!(c2s(&mut data, &chat(3, 0b111 << 17), P, P, true).is_none());
    }
}
