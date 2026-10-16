pub mod packets;

use std::collections::{HashSet, VecDeque};

use mc_protocol::{
    packet::{PacketId, RawPacket, UncompressedPacket},
    ser::Serialize,
    varint::VarInt,
};

use super::{PacketIdReader, VersionProtocol, decode};
use crate::{
    config,
    events::{ClientId, PerClient},
    protocols::{ClientBoundEvent, ClientDisconnectEvent, ServerBoundEvent},
};
use packets::{
    c2s::{self, game::LastSeenEntry},
    s2c,
};

/// Vanilla servers never issue negative teleport ids.
const TELEPORT_ID: i32 = -1;
/// Entries in a `LastSeenMessagesTracker`.
const LAST_SEEN: usize = 5;
/// `ChatVisiblity.HIDDEN`, for which `tryHandleChat` skips `validateAndUpdate`.
const CHAT_HIDDEN: i32 = 2;

pub struct VersionData {
    pub position: s2c::game::Position,
    /// Teleports each client got and has not acked yet, in the order it got them.
    pub unacked: PerClient<VecDeque<Teleport>>,
    pub pings: Vec<PingSync>,
    pub teleports: Vec<TeleportSync>,
    pub keep_alives: Vec<KeepAliveSync>,
    pub chat: ChatSync,
    /// The latest chat time stamp the server got, from `Instant.EPOCH` like `lastChatTimeStamp`.
    pub chat_time_stamp: i64,
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
                dismount_vehicle: false,
            },
            unacked: Default::default(),
            pings: vec![],
            teleports: vec![],
            keep_alives: vec![],
            chat: ChatSync::default(),
            chat_time_stamp: 0,
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
        self.chat.active = new_active;
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
            s2c::game::Login::PACKET_ID => self.chat.restart(),
            s2c::game::PlayerChat::PACKET_ID => {
                let chat: s2c::game::PlayerChat = decode(packet, self.threshold)?;
                // `toLastSeenEntry` skips unsigned and system messages.
                if !chat.signature.0.is_empty() && !chat.sender.is_nil() {
                    self.chat.track(
                        LastSeenEntry {
                            profile_id: chat.sender,
                            signature: chat.signature,
                        },
                        both_online,
                    );
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
        if is_active {
            self.chat.active = client_id;
        }

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
                let signed = !command.argument_signatures.is_empty();
                let raised = self.order_chat(&mut command.time_stamp, client_id, is_active, signed);
                if self.sync_last_seen(&mut command.last_seen, client_id, is_active, signed)
                    || raised
                {
                    return Ok(Some(self.rewrite(&command)?));
                }
            }
            c2s::game::Chat::PACKET_ID => {
                if is_active && let Ok(text) = decode::<String>(packet, self.threshold) {
                    config::report_chat(text, false);
                }
                let mut chat: c2s::game::Chat = decode(packet, self.threshold)?;
                let signed = !chat.signature.0.is_empty();
                let raised = self.order_chat(&mut chat.time_stamp, client_id, is_active, signed);
                if self.sync_last_seen(&mut chat.last_seen, client_id, is_active, signed) || raised
                {
                    return Ok(Some(self.rewrite(&chat)?));
                }
            }
            c2s::game::ChatAck::PACKET_ID => {
                let ack: c2s::game::ChatAck = decode(packet, self.threshold)?;
                if let Some(last_seen) =
                    self.chat
                        .update(client_id, &ack.last_seen, is_active, false)
                {
                    return Ok(Some(self.rewrite(&c2s::game::ChatAck { last_seen })?));
                }
            }
            c2s::game::ClientInformation::PACKET_ID => {
                let info: c2s::game::ClientInformation = decode(packet, self.threshold)?;
                let hidden = info.chat_visibility.0 == CHAT_HIDDEN;
                self.chat.hidden[client_id] = hidden;
                if is_active {
                    self.chat.server_hidden = hidden;
                }
            }
            _ => {}
        };

        Ok(None)
    }

    /// Moves a chat's last seen update onto the server's validator, returning whether it changed.
    fn sync_last_seen(
        &mut self,
        last_seen: &mut c2s::game::LastSeen,
        client_id: ClientId,
        is_active: bool,
        signed: bool,
    ) -> bool {
        let Some(synced) = self.chat.update(client_id, last_seen, is_active, true) else {
            return false;
        };
        if signed {
            crate::diag::warn(
                "chat",
                format!(
                    "{client_id:?} sent signed chat with a last seen list the server rejects, syncing it breaks the signature"
                ),
            );
        }
        *last_seen = synced;
        true
    }

    /// Raises a time stamp behind the last one the server got, which `updateChatOrder` kicks for.
    fn order_chat(
        &mut self,
        time_stamp: &mut i64,
        client_id: ClientId,
        is_active: bool,
        signed: bool,
    ) -> bool {
        if !is_active {
            return false;
        }
        if *time_stamp >= self.chat_time_stamp {
            self.chat_time_stamp = *time_stamp;
            return false;
        }
        if signed {
            crate::diag::warn(
                "chat",
                format!(
                    "{client_id:?} sent signed chat behind the server's clock, raising it breaks the signature"
                ),
            );
        }
        *time_stamp = self.chat_time_stamp;
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

/// A replica of `LastSeenMessagesValidator`.
#[derive(Debug, Default, Clone)]
pub struct Validator {
    last_seen: Vec<LastSeenEntry>,
    pending: Vec<LastSeenEntry>,
}

impl Validator {
    /// `validateAndUpdate`, returning whether it found no error.
    fn validate_and_update(&mut self, update: &c2s::game::LastSeen) -> bool {
        let entries = &update.last_seen;
        let mut valid = entries.len() >= self.last_seen.len();
        // `calculateIndices`: the old list counts back from -1, pending messages up from 0.
        let mut indices = vec![None; entries.len()];
        for (i, entry) in self.last_seen.iter().enumerate().rev() {
            if let Some(at) = entries.iter().position(|e| e == entry) {
                indices[at] = Some(-(i as i64) - 1);
            }
        }
        let mut received = None;
        for (i, entry) in self.pending.iter().enumerate() {
            if let Some(at) = entries.iter().position(|e| e == entry) {
                indices[at] = Some(i as i64);
            }
            if update.last_received.as_ref() == Some(entry) {
                received = Some(i as i64);
            }
        }
        let mut latest = i64::MIN;
        for index in indices.iter().rev() {
            match *index {
                Some(index) if index < latest => valid = false,
                Some(index) => latest = index,
                None => valid = false,
            }
        }
        if update.last_received.is_some() {
            match received {
                Some(index) if index >= latest => latest = index,
                _ => valid = false,
            }
        }
        if latest >= 0 {
            self.pending.drain(..=latest as usize);
        }
        let mut profiles = HashSet::new();
        valid &= entries.iter().all(|e| profiles.insert(e.profile_id));
        self.last_seen = entries.clone();
        valid
    }

    fn accepts(&self, update: &c2s::game::LastSeen) -> bool {
        self.clone().validate_and_update(update)
    }

    /// An update this accepts: the list it has stays, plus what the client saw since.
    fn repair(&self, update: &c2s::game::LastSeen) -> c2s::game::LastSeen {
        let mut synced = c2s::game::LastSeen {
            last_seen: update.last_seen.clone(),
            last_received: None,
        };
        if !self.accepts(&synced) {
            synced.last_seen = self.last_seen.clone();
            for entry in self.pending.iter().filter(|e| update.last_seen.contains(e)) {
                push(&mut synced.last_seen, entry.clone());
            }
        }
        let with_received = c2s::game::LastSeen {
            last_received: update.last_received.clone(),
            ..synced.clone()
        };
        if self.accepts(&with_received) {
            with_received
        } else {
            synced
        }
    }
}

/// `LastSeenMessagesTracker.push`: newest first, one entry per sender.
fn push(entries: &mut Vec<LastSeenEntry>, entry: LastSeenEntry) {
    if let Some(i) = entries
        .iter()
        .position(|e| e.profile_id == entry.profile_id)
    {
        entries.remove(i);
    } else if entries.len() == LAST_SEEN {
        entries.pop();
    }
    entries.insert(0, entry);
}

/// The server's validator, and the one each client would have had if it was active all along.
#[derive(Debug)]
pub struct ChatSync {
    server: Validator,
    clients: PerClient<Validator>,
    /// Whether the server's and each client's own chat visibility is hidden.
    server_hidden: bool,
    hidden: PerClient<bool>,
    active: ClientId,
}

impl Default for ChatSync {
    fn default() -> Self {
        Self {
            server: Validator::default(),
            clients: Default::default(),
            server_hidden: false,
            hidden: PerClient::splat(false),
            active: ClientId::Primary,
        }
    }
}

impl ChatSync {
    /// Each `ClientboundLoginPacket`, a backend switch behind a proxy too, has a fresh validator.
    pub fn restart(&mut self) {
        self.server = Validator::default();
        self.clients = Default::default();
    }

    /// `addPendingMessage`, mirrored for each client still online.
    pub fn track(&mut self, entry: LastSeenEntry, both_online: bool) {
        if both_online {
            self.clients[self.active.opposite()]
                .pending
                .push(entry.clone());
        } else {
            self.clients[self.active.opposite()] = Validator::default();
        }
        self.clients[self.active].pending.push(entry.clone());
        self.server.pending.push(entry);
    }

    /// Records a client's update and returns the one to relay if only the switch makes the server reject it.
    pub fn update(
        &mut self,
        client: ClientId,
        update: &c2s::game::LastSeen,
        relay: bool,
        chat: bool,
    ) -> Option<c2s::game::LastSeen> {
        let own = (chat && self.hidden[client]) || self.clients[client].validate_and_update(update);
        if !relay || (chat && self.server_hidden) {
            return None;
        }
        // An update the client's own validator rejects gets the vanilla kick.
        if !own || self.server.accepts(update) {
            self.server.validate_and_update(update);
            return None;
        }
        let synced = self.server.repair(update);
        self.server.validate_and_update(&synced);
        Some(synced)
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
    use mc_protocol::ser::{ByteArray, Deserialize};

    use super::*;

    const P: ClientId = ClientId::Primary;
    const S: ClientId = ClientId::Secondary;

    fn c2s<T: PacketId + Serialize>(
        data: &mut VersionData,
        packet: &T,
        client: ClientId,
        active: ClientId,
        both_online: bool,
    ) -> Option<ServerBoundEvent> {
        let packet = UncompressedPacket::from_packet(packet)
            .unwrap()
            .to_raw_packet_compressed(data.threshold)
            .unwrap();
        data.handle_c2s(&packet, client, client == active, both_online)
    }

    fn last_seen() -> c2s::game::LastSeen {
        c2s::game::LastSeen {
            last_seen: vec![],
            last_received: None,
        }
    }

    fn chat_at(time_stamp: i64) -> c2s::game::Chat {
        c2s::game::Chat {
            message: "hi".into(),
            time_stamp,
            salt: 0,
            signature: ByteArray::default(),
            signed_preview: false,
            last_seen: last_seen(),
        }
    }

    /// A premium client signs even through the proxy.
    fn command_at(time_stamp: i64) -> c2s::game::ChatCommand {
        c2s::game::ChatCommand {
            command: "msg a b".into(),
            time_stamp,
            salt: 1,
            argument_signatures: vec![c2s::game::ArgumentSignature {
                name: "message".into(),
                signature: ByteArray(vec![1; 256]),
            }],
            signed_preview: false,
            last_seen: last_seen(),
        }
    }

    /// The time stamp of a rewritten chat or command, which both put right after the text.
    fn raised(data: &VersionData, event: Option<ServerBoundEvent>) -> i64 {
        let Some(ServerBoundEvent::Rewrite(packet)) = event else {
            panic!("expected a rewrite");
        };
        let packet = packet.uncompress(data.threshold).unwrap();
        let mut payload = std::io::Cursor::new(&packet.payload);
        String::deserialize(&mut payload).unwrap();
        i64::deserialize(&mut payload).unwrap()
    }

    #[test]
    fn lagging_clock_after_failover_is_raised() {
        let mut data = VersionData::new();
        assert!(c2s(&mut data, &chat_at(10_000), P, P, true).is_none());
        data.handle_client_disconnect(S);
        let event = c2s(&mut data, &chat_at(4_000), S, S, false);
        assert_eq!(raised(&data, event), 10_000);
        let event = c2s(&mut data, &command_at(5_000), S, S, false);
        assert_eq!(raised(&data, event), 10_000);
        assert!(c2s(&mut data, &chat_at(12_000), S, S, false).is_none());
    }

    #[test]
    fn inactive_chat_leaves_chat_order() {
        let mut data = VersionData::new();
        assert!(c2s(&mut data, &chat_at(1_000), P, P, true).is_none());
        assert!(c2s(&mut data, &chat_at(5_000), S, P, true).is_none());
        assert!(c2s(&mut data, &command_at(2_000), P, P, true).is_none());
        // `updateChatOrder` only rejects `isBefore`.
        assert!(c2s(&mut data, &chat_at(2_000), P, P, true).is_none());
    }

    fn entry(sender: u8, signature: u8) -> LastSeenEntry {
        LastSeenEntry {
            profile_id: uuid::Uuid::from_u128(sender as u128),
            signature: ByteArray(vec![signature; 256]),
        }
    }

    /// `ClientPacketListener`'s acks with a `LastSeenMessagesTracker(5)`, as vanilla writes them.
    struct VanillaClient {
        status: [Option<LastSeenEntry>; 5],
        size: usize,
        last_unacknowledged: Option<LastSeenEntry>,
        unacknowledged: u32,
    }

    impl VanillaClient {
        fn new() -> Self {
            Self {
                status: Default::default(),
                size: 0,
                last_unacknowledged: None,
                unacknowledged: 0,
            }
        }

        fn push(&mut self, entry: LastSeenEntry) {
            let mut carried = Some(entry.clone());
            for i in 0..self.size {
                let current = self.status[i].take();
                self.status[i] = carried;
                carried = current;
                if carried.as_ref().unwrap().profile_id == entry.profile_id {
                    carried = None;
                    break;
                }
            }
            if let Some(carried) = carried
                && self.size < self.status.len()
            {
                self.status[self.size] = Some(carried);
                self.size += 1;
            }
        }

        /// `markMessageAsProcessed`, returning the `ServerboundChatAckPacket` once one is due.
        fn process(&mut self, entry: LastSeenEntry, shown: bool) -> Option<c2s::game::ChatAck> {
            if shown {
                self.push(entry);
                self.last_unacknowledged = None;
            } else {
                self.last_unacknowledged = Some(entry);
            }
            let count = self.unacknowledged;
            self.unacknowledged += 1;
            (count > 64).then(|| c2s::game::ChatAck {
                last_seen: self.acknowledge(),
            })
        }

        /// `generateMessageAcknowledgements`, which keeps the last hidden message.
        fn acknowledge(&mut self) -> c2s::game::LastSeen {
            self.unacknowledged = 0;
            c2s::game::LastSeen {
                last_seen: self.status[..self.size].iter().flatten().cloned().collect(),
                last_received: self.last_unacknowledged.clone(),
            }
        }
    }

    /// `ServerGamePacketListenerImpl` with its `LastSeenMessagesValidator`, as vanilla writes them.
    #[derive(Default)]
    struct VanillaServer {
        last_seen: Vec<LastSeenEntry>,
        pending: Vec<LastSeenEntry>,
        hidden: bool,
    }

    impl VanillaServer {
        fn validate_and_update(&mut self, update: &c2s::game::LastSeen) -> Vec<&'static str> {
            let mut errors = vec![];
            let entries = &update.last_seen;
            let received = update.last_received.as_ref();
            let mut latest = i32::MIN;
            if entries.len() < self.last_seen.len() {
                errors.push("previously present messages removed from context");
            }
            let mut indices = vec![i32::MIN; entries.len()];
            for i in (0..self.last_seen.len()).rev() {
                if let Some(at) = entries.iter().position(|e| *e == self.last_seen[i]) {
                    indices[at] = -(i as i32) - 1;
                }
            }
            let mut received_index = i32::MIN;
            for (i, pending) in self.pending.iter().enumerate() {
                if let Some(at) = entries.iter().position(|e| e == pending) {
                    indices[at] = i as i32;
                }
                if Some(pending) == received {
                    received_index = i as i32;
                }
            }
            for i in (0..entries.len()).rev() {
                let index = indices[i];
                if index != i32::MIN {
                    if index < latest {
                        errors.push("messages received out of order");
                    } else {
                        latest = index;
                    }
                } else {
                    errors.push("unknown message");
                }
            }
            if received.is_some() {
                if received_index != i32::MIN && received_index >= latest {
                    latest = received_index;
                } else {
                    errors.push("unknown message");
                }
            }
            if latest >= 0 {
                self.pending.drain(..=latest as usize);
            }
            let mut profiles = HashSet::new();
            if !entries.iter().all(|e| profiles.insert(e.profile_id)) {
                errors.push("multiple entries for single profile");
            }
            self.last_seen = entries.clone();
            errors.sort();
            errors.dedup();
            errors
        }

        /// `handleChat`, `handleChatCommand`, `handleChatAck`; `Err` is the `chat_validation_failed` reason.
        fn receive(&mut self, packet: &RawPacket) -> Result<(), String> {
            let packet = packet.uncompress(None).unwrap();
            let update = match packet.packet_id {
                c2s::game::Chat::PACKET_ID | c2s::game::ChatCommand::PACKET_ID if self.hidden => {
                    return Ok(());
                }
                c2s::game::Chat::PACKET_ID => {
                    packet
                        .deserialize_payload::<c2s::game::Chat>()
                        .unwrap()
                        .last_seen
                }
                c2s::game::ChatCommand::PACKET_ID => {
                    packet
                        .deserialize_payload::<c2s::game::ChatCommand>()
                        .unwrap()
                        .last_seen
                }
                c2s::game::ChatAck::PACKET_ID => {
                    packet
                        .deserialize_payload::<c2s::game::ChatAck>()
                        .unwrap()
                        .last_seen
                }
                c2s::game::ClientInformation::PACKET_ID => {
                    let info: c2s::game::ClientInformation = packet.deserialize_payload().unwrap();
                    self.hidden = info.chat_visibility.0 == 2;
                    return Ok(());
                }
                _ => return Ok(()),
            };
            let errors = self.validate_and_update(&update);
            if errors.is_empty() {
                Ok(())
            } else {
                Err(errors.join(","))
            }
        }
    }

    /// Two vanilla clients behind the proxy and a vanilla server.
    struct Lab {
        data: VersionData,
        server: VanillaServer,
        clients: PerClient<VanillaClient>,
        active: ClientId,
        online: PerClient<bool>,
    }

    const SHOWN: [Option<bool>; 2] = [Some(true), Some(true)];

    impl Lab {
        fn new() -> Self {
            Self {
                data: VersionData::new(),
                server: VanillaServer::default(),
                clients: PerClient::new(VanillaClient::new(), VanillaClient::new()),
                active: P,
                online: PerClient::splat(true),
            }
        }

        fn both_online(&self) -> bool {
            self.online == PerClient::splat(true)
        }

        /// A signed message each client shows, hides, or handles later (`None`); returns the acks sent.
        fn message(
            &mut self,
            entry: &LastSeenEntry,
            shown: [Option<bool>; 2],
        ) -> Vec<Result<bool, String>> {
            self.server.pending.push(entry.clone());
            let packet = s2c::game::PlayerChat {
                previous_signature: None,
                sender: entry.profile_id,
                signature: entry.signature.clone(),
            };
            let packet = UncompressedPacket::from_packet(&packet)
                .unwrap()
                .to_raw_packet_compressed(None)
                .unwrap();
            let both_online = self.both_online();
            self.data.handle_s2c(&packet, both_online);
            [P, S]
                .into_iter()
                .zip(shown)
                .filter_map(|(client, shown)| self.process(client, entry, shown?))
                .collect()
        }

        fn process(
            &mut self,
            client: ClientId,
            entry: &LastSeenEntry,
            shown: bool,
        ) -> Option<Result<bool, String>> {
            if !self.online[client] {
                return None;
            }
            let ack = self.clients[client].process(entry.clone(), shown)?;
            Some(self.send(client, &ack))
        }

        /// Relays a packet like the controller; returns whether it was rewritten, or the kick.
        fn send<T: PacketId + Serialize>(
            &mut self,
            client: ClientId,
            packet: &T,
        ) -> Result<bool, String> {
            let packet = UncompressedPacket::from_packet(packet)
                .unwrap()
                .to_raw_packet_compressed(None)
                .unwrap();
            let both_online = self.both_online();
            let event = self
                .data
                .handle_c2s(&packet, client, client == self.active, both_online);
            if client != self.active {
                return Ok(false);
            }
            let (packet, rewritten) = match event {
                Some(ServerBoundEvent::Rewrite(packet)) => (packet, true),
                None => (packet, false),
                _ => panic!("unexpected event"),
            };
            self.server.receive(&packet).map(|()| rewritten)
        }

        fn chat(&mut self, client: ClientId) -> Result<bool, String> {
            let last_seen = self.clients[client].acknowledge();
            self.send(
                client,
                &c2s::game::Chat {
                    last_seen,
                    ..chat_at(0)
                },
            )
        }

        fn command(&mut self, client: ClientId) -> Result<bool, String> {
            let last_seen = self.clients[client].acknowledge();
            self.send(
                client,
                &c2s::game::ChatCommand {
                    last_seen,
                    ..command_at(0)
                },
            )
        }

        fn chat_visibility(&mut self, client: ClientId, visibility: i32) {
            let info = c2s::game::ClientInformation {
                language: "en_us".into(),
                view_distance: 12,
                chat_visibility: VarInt(visibility),
            };
            assert_eq!(self.send(client, &info), Ok(false));
        }

        fn failover(&mut self) {
            self.online[self.active] = false;
            self.active = self.active.opposite();
            self.data.handle_client_disconnect(self.active);
        }
    }

    #[test]
    fn same_view_passes_unchanged() {
        let mut lab = Lab::new();
        for i in 0..8 {
            lab.message(&entry(i % 3 + 1, i), SHOWN);
        }
        assert_eq!(lab.chat(P), Ok(false));
        lab.message(&entry(4, 20), SHOWN);
        lab.message(&entry(1, 21), [Some(false), Some(false)]);
        assert_eq!(lab.command(P), Ok(false));
        lab.message(&entry(5, 22), SHOWN);
        lab.failover();
        assert_eq!(lab.chat(S), Ok(false));
        lab.message(&entry(2, 23), SHOWN);
        assert_eq!(lab.command(S), Ok(false));
        let acks: Vec<_> = (0..70)
            .flat_map(|i| lab.message(&entry(i % 7 + 1, 100 + i), SHOWN))
            .collect();
        assert_eq!(acks, vec![Ok(false)]);
        assert_eq!(lab.chat(S), Ok(false));
    }

    #[test]
    fn message_hidden_by_new_active_stays_acked() {
        let mut lab = Lab::new();
        lab.message(&entry(1, 1), SHOWN);
        lab.message(&entry(2, 2), [Some(true), Some(false)]);
        lab.message(&entry(3, 3), [Some(true), Some(false)]);
        assert_eq!(lab.chat(P), Ok(false));
        lab.failover();
        assert_eq!(lab.chat(S), Ok(true));
        lab.message(&entry(4, 4), SHOWN);
        assert_eq!(lab.command(S), Ok(true));
    }

    #[test]
    fn message_hidden_by_old_active_is_not_acked() {
        let mut lab = Lab::new();
        lab.message(&entry(1, 1), [Some(false), Some(true)]);
        assert_eq!(lab.chat(P), Ok(false));
        lab.failover();
        assert_eq!(lab.chat(S), Ok(true));
        lab.message(&entry(2, 2), SHOWN);
        assert_eq!(lab.command(S), Ok(true));
    }

    #[test]
    fn lagging_client_keeps_acks_server_got() {
        let mut lab = Lab::new();
        let late = entry(1, 2);
        lab.message(&entry(1, 1), SHOWN);
        lab.message(&late, [Some(true), None]);
        assert_eq!(lab.chat(P), Ok(false));
        lab.failover();
        assert_eq!(lab.chat(S), Ok(true));
        assert_eq!(lab.process(S, &late, true), None);
        assert_eq!(lab.chat(S), Ok(false));
    }

    #[test]
    fn chat_ack_after_failover_matches_server() {
        let mut lab = Lab::new();
        lab.message(&entry(2, 1), [Some(true), Some(false)]);
        lab.message(&entry(3, 2), [Some(true), Some(false)]);
        assert_eq!(lab.chat(P), Ok(false));
        lab.failover();
        let acks: Vec<_> = (0..66)
            .flat_map(|i| lab.message(&entry(1, 10 + i), SHOWN))
            .collect();
        assert_eq!(acks, vec![Ok(true)]);
    }

    #[test]
    fn resent_hidden_message_gets_the_vanilla_kick() {
        let mut lab = Lab::new();
        lab.message(&entry(1, 1), [Some(false), Some(false)]);
        assert_eq!(lab.chat(P), Ok(false));
        assert_eq!(lab.chat(P), Err("unknown message".into()));

        let mut lab = Lab::new();
        lab.message(&entry(1, 1), [Some(false), Some(false)]);
        assert_eq!(lab.chat(P), Ok(false));
        lab.failover();
        // The secondary sends it for the first time, its own server would take it.
        assert_eq!(lab.chat(S), Ok(true));
        assert_eq!(lab.chat(S), Err("unknown message".into()));
    }

    #[test]
    fn hidden_chat_is_not_validated() {
        let mut lab = Lab::new();
        lab.message(&entry(1, 1), SHOWN);
        lab.message(&entry(2, 2), [Some(true), Some(false)]);
        lab.message(&entry(3, 3), [Some(true), Some(false)]);
        assert_eq!(lab.chat(P), Ok(false));
        lab.chat_visibility(P, CHAT_HIDDEN);
        lab.failover();
        assert_eq!(lab.chat(S), Ok(false));
        lab.chat_visibility(S, 0);
        lab.message(&entry(4, 4), SHOWN);
        assert_eq!(lab.chat(S), Ok(true));
    }

    #[test]
    fn inactive_updates_never_reach_server() {
        let mut lab = Lab::new();
        lab.message(&entry(1, 1), [Some(true), Some(false)]);
        assert_eq!(lab.chat(S), Ok(false));
        assert_eq!(lab.chat(P), Ok(false));
        assert_eq!(lab.chat(P), Ok(false));
    }

    fn uuid_bytes(first: u8) -> Vec<u8> {
        (first..first + 16).collect()
    }

    /// `LastSeenMessages.Update` with two entries and a last received one, written by hand.
    fn last_seen_bytes() -> (Vec<u8>, c2s::game::LastSeen) {
        let bytes = [
            vec![2],
            uuid_bytes(0x01),
            vec![3, 0xaa, 0xbb, 0xcc],
            uuid_bytes(0x40),
            vec![2, 0xdd, 0xee],
            vec![1],
            uuid_bytes(0x80),
            vec![1, 0xff],
        ]
        .concat();
        let uuid = |first: u8| uuid::Uuid::from_slice(&uuid_bytes(first)).unwrap();
        let last_seen = c2s::game::LastSeen {
            last_seen: vec![
                LastSeenEntry {
                    profile_id: uuid(0x01),
                    signature: ByteArray(vec![0xaa, 0xbb, 0xcc]),
                },
                LastSeenEntry {
                    profile_id: uuid(0x40),
                    signature: ByteArray(vec![0xdd, 0xee]),
                },
            ],
            last_received: Some(LastSeenEntry {
                profile_id: uuid(0x80),
                signature: ByteArray(vec![0xff]),
            }),
        };
        (bytes, last_seen)
    }

    fn assert_vanilla_bytes<T: PacketId + Serialize + Deserialize>(id: i32, payload: &[u8]) -> T {
        let packet = UncompressedPacket::new(id, payload.to_vec())
            .to_raw_packet()
            .unwrap();
        let decoded: T = super::super::decode(&packet, None).unwrap();
        let encoded = UncompressedPacket::from_packet(&decoded).unwrap();
        assert_eq!(
            (encoded.packet_id, encoded.payload.as_slice()),
            (id, payload)
        );
        decoded
    }

    #[test]
    fn chat_packets_match_vanilla_bytes() {
        let (last_seen_bytes, last_seen) = last_seen_bytes();
        let time_stamp = 0x0000_0183_0000_1234_i64;

        let ack: c2s::game::ChatAck = assert_vanilla_bytes(3, &last_seen_bytes);
        assert_eq!(ack.last_seen, last_seen);

        let chat = [
            vec![2, b'h', b'i'],
            time_stamp.to_be_bytes().to_vec(),
            7_i64.to_be_bytes().to_vec(),
            vec![2, 0x51, 0x52],
            vec![1],
            last_seen_bytes.clone(),
        ]
        .concat();
        let chat: c2s::game::Chat = assert_vanilla_bytes(5, &chat);
        assert_eq!(
            (chat.message.as_str(), chat.time_stamp, chat.salt),
            ("hi", time_stamp, 7)
        );
        assert_eq!(chat.signature, ByteArray(vec![0x51, 0x52]));
        assert!(chat.signed_preview);
        assert_eq!(chat.last_seen, last_seen);

        let command = [
            vec![5, b'm', b's', b'g', b' ', b'a'],
            time_stamp.to_be_bytes().to_vec(),
            (-2_i64).to_be_bytes().to_vec(),
            vec![1, 7],
            b"message".to_vec(),
            vec![2, 0x61, 0x62],
            vec![0],
            last_seen_bytes,
        ]
        .concat();
        let command: c2s::game::ChatCommand = assert_vanilla_bytes(4, &command);
        assert_eq!(
            (command.command.as_str(), command.time_stamp, command.salt),
            ("msg a", time_stamp, -2)
        );
        assert_eq!(command.argument_signatures.len(), 1);
        assert_eq!(command.argument_signatures[0].name, "message");
        assert_eq!(
            command.argument_signatures[0].signature,
            ByteArray(vec![0x61, 0x62])
        );
        assert!(!command.signed_preview);
        assert_eq!(command.last_seen, last_seen);
    }

    #[test]
    fn player_chat_header_matches_vanilla_bytes() {
        let payload = [
            vec![1, 2, 0x31, 0x32],
            uuid_bytes(0x01),
            vec![3, 0x41, 0x42, 0x43],
            // `SignedMessageBody`, unsigned content, `FilterMask` and `ChatType.BoundNetwork`.
            vec![2, b'h', b'i', 0],
            0_i64.to_be_bytes().to_vec(),
            0_i64.to_be_bytes().to_vec(),
            vec![0, 0, 0, 0, 2, b'"', b'"', 0],
        ]
        .concat();
        let packet = UncompressedPacket::new(51, payload)
            .to_raw_packet()
            .unwrap();
        let chat: s2c::game::PlayerChat = super::super::decode(&packet, None).unwrap();
        assert_eq!(chat.previous_signature, Some(ByteArray(vec![0x31, 0x32])));
        assert_eq!(
            chat.sender,
            uuid::Uuid::from_slice(&uuid_bytes(0x01)).unwrap()
        );
        assert_eq!(chat.signature, ByteArray(vec![0x41, 0x42, 0x43]));
    }
}
