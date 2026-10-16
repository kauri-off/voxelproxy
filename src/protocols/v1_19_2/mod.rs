pub mod packets;

use std::collections::VecDeque;

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
                let signed = !command.argument_signatures.is_empty();
                if self.order_chat(&mut command.time_stamp, client_id, is_active, signed) {
                    return Ok(Some(self.rewrite(&command)?));
                }
            }
            c2s::game::Chat::PACKET_ID => {
                if is_active && let Ok(text) = decode::<String>(packet, self.threshold) {
                    config::report_chat(text, false);
                }
                let mut chat: c2s::game::Chat = decode(packet, self.threshold)?;
                let signed = !chat.signature.0.is_empty();
                if self.order_chat(&mut chat.time_stamp, client_id, is_active, signed) {
                    return Ok(Some(self.rewrite(&chat)?));
                }
            }
            _ => {}
        };

        Ok(None)
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
}
