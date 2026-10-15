pub mod packets;

use std::collections::VecDeque;

use mc_protocol::{
    packet::{PacketId, RawPacket, UncompressedPacket},
    ser::Serialize,
    varint::VarInt,
};
use uuid::Uuid;

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
/// How far an answer takes a task; the server finishes it on `DONE`.
const ACCEPTED: u8 = 1;
const DOWNLOADED: u8 = 2;
const DONE: u8 = 3;

#[derive(PartialEq, Debug, Clone, Copy)]
pub enum State {
    Login,
    Configuration,
    Game,
}

pub struct VersionData {
    /// Shared by both clients: they read the same server stream.
    pub state_in: State,
    pub state_out: PerClient<State>,
    pub position: s2c::game::Position,
    /// Teleports each client got and has not acked yet, in the order it got them.
    pub unacked: PerClient<VecDeque<Teleport>>,
    pub pings: Vec<PingSync>,
    pub teleports: Vec<TeleportSync>,
    pub keep_alives: Vec<KeepAliveSync>,
    pub chat: ChatSync,
    pub chunk_batches: Vec<ChunkBatchSync>,
    /// Configuration tasks the server waits to be answered, see `ServerConfigurationPacketListenerImpl`.
    pub tasks: Vec<TaskSync>,
    /// Counts tracked pings, teleports, keep-alives, tasks and chunk batches in server stream order.
    pub seq: u64,
    /// Phase switches each client acked.
    pub phases: PerClient<u32>,
    /// Phase switches the server got an ack for.
    pub server_phases: u32,
    /// Phase switches in the server stream, counting LoginSuccess.
    pub stream_phases: u32,
    /// What the inactive client sent from an ack the server has not got yet.
    pub ahead: Vec<RawPacket>,
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
            // The controller starts after LoginSuccess was relayed to both clients.
            state_in: State::Configuration,
            state_out: PerClient::splat(State::Login),
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
            unacked: Default::default(),
            pings: vec![],
            teleports: vec![],
            keep_alives: vec![],
            chat: ChatSync::default(),
            chunk_batches: vec![],
            tasks: vec![],
            seq: 0,
            phases: PerClient::splat(0),
            server_phases: 0,
            stream_phases: 1,
            ahead: vec![],
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
        let state = self.state_out[client_id];
        let behind = is_active && self.phases[client_id] < self.server_phases;
        let event = match state {
            State::Login => self.handle_c2s_login(packet, client_id),
            State::Configuration => {
                self.handle_c2s_configuration(packet, client_id, is_active, both_online)
            }
            State::Game => self.handle_c2s_game(packet, client_id, is_active, both_online),
        }
        .unwrap_or_default();
        if self.state_out[client_id] != state {
            self.phases[client_id] += 1;
        }

        if behind {
            // The server already got this phase from the old active client.
            return Some(match event {
                Some(ServerBoundEvent::PhaseAcked) => ServerBoundEvent::PhaseCaughtUp,
                _ => ServerBoundEvent::SkipRelay,
            });
        }
        if is_active && self.phases[client_id] > self.server_phases {
            self.server_phases = self.phases[client_id];
            self.ahead.clear();
            // The listener that sent these keep-alives and tasks is gone.
            self.keep_alives.clear();
            self.tasks.clear();
            // Pings sent after the phase switch are answered to the new listener.
            self.pings.retain(|p| p.phase >= self.server_phases);
        } else if !is_active
            && self.phases[client_id] > self.server_phases
            && !matches!(event, Some(ServerBoundEvent::SkipRelay))
        {
            self.ahead.push(packet.clone());
        }
        event
    }

    fn handle_s2c(&mut self, packet: &RawPacket, both_online: bool) -> Option<ClientBoundEvent> {
        let event = match self.state_in {
            State::Login => Ok(None),
            State::Configuration => self.handle_s2c_configuration(packet, both_online),
            State::Game => self.handle_s2c_game(packet, both_online),
        }
        .unwrap_or_default();
        if matches!(event, Some(ClientBoundEvent::PhaseSwitch)) {
            self.stream_phases += 1;
        }
        event
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
            // An answer to a listener the server has not reached waits in `ahead`.
            if !t.sent && t.phase == self.server_phases {
                let tx = match t.state {
                    State::Configuration => {
                        UncompressedPacket::from_packet(&c2s::configuration::Pong { id: t.id })
                    }
                    _ => UncompressedPacket::from_packet(&c2s::game::Pong { id: t.id }),
                };
                replies.push((
                    t.seq,
                    tx.unwrap()
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
                let tx = match t.state {
                    State::Configuration => {
                        UncompressedPacket::from_packet(&c2s::configuration::KeepAlive { id: t.id })
                    }
                    _ => UncompressedPacket::from_packet(&c2s::game::KeepAlive { id: t.id }),
                };
                replies.push((
                    t.seq,
                    tx.unwrap()
                        .to_raw_packet_compressed(self.threshold)
                        .unwrap(),
                ));
            }
            false
        });

        self.chunk_batches.retain_mut(|t| {
            let Some(answer) = t.answers[new_active].take() else {
                return true;
            };
            if !t.sent {
                replies.push((t.seq, answer));
            }
            false
        });

        for task in &mut self.tasks {
            let seq = task.seq;
            replies.extend(task.catch_up(new_active).into_iter().map(|p| (seq, p)));
        }
        self.tasks.retain(|t| !t.done(new_active));

        // Vanilla clients answer these in server order; keep-alives only roughly (netty thread).
        replies.sort_by_key(|(seq, _)| *seq);
        let mut packets: Vec<_> = replies.into_iter().map(|(_, packet)| packet).collect();

        let ahead = std::mem::take(&mut self.ahead);
        if self.phases[new_active] > self.server_phases {
            self.server_phases = self.phases[new_active];
            self.pings.retain(|p| p.phase >= self.server_phases);
            self.keep_alives.clear();
            self.tasks.clear();
            packets.extend(ahead);
        }

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
        &mut self.state_out[client_id]
    }

    fn handle_s2c_configuration(
        &mut self,
        packet: &RawPacket,
        both_online: bool,
    ) -> anyhow::Result<Option<ClientBoundEvent>> {
        let id = self.ids.read(packet, self.threshold)?;

        match id {
            s2c::configuration::FinishConfiguration::PACKET_ID => {
                self.state_in = State::Game;
                self.chat.restart_server();
                return Ok(Some(ClientBoundEvent::PhaseSwitch));
            }
            s2c::configuration::Ping::PACKET_ID if both_online => {
                let ping: s2c::configuration::Ping = decode(packet, self.threshold)?;
                self.seq += 1;
                self.pings.push(PingSync::new(
                    ping.id,
                    State::Configuration,
                    self.stream_phases,
                    self.seq,
                ));
            }
            s2c::configuration::KeepAlive::PACKET_ID if both_online => {
                let keep_alive: s2c::configuration::KeepAlive = decode(packet, self.threshold)?;
                self.seq += 1;
                self.keep_alives.push(KeepAliveSync::new(
                    keep_alive.id,
                    State::Configuration,
                    self.seq,
                ));
            }
            s2c::configuration::SelectKnownPacks::PACKET_ID => {
                self.track_task(Task::KnownPacks, both_online);
            }
            s2c::configuration::ResourcePackPush::PACKET_ID => {
                let push: s2c::configuration::ResourcePackPush = decode(packet, self.threshold)?;
                // A vanilla client drops a pack pushed again under its id without answering for it.
                self.tasks.retain(|t| t.task != Task::ResourcePack(push.id));
                self.track_task(Task::ResourcePack(push.id), both_online);
            }
            _ => {}
        }
        Ok(None)
    }
    fn handle_s2c_game(
        &mut self,
        packet: &RawPacket,
        both_online: bool,
    ) -> anyhow::Result<Option<ClientBoundEvent>> {
        let id = self.ids.read(packet, self.threshold)?;

        match id {
            s2c::game::StartConfiguration::PACKET_ID => {
                self.state_in = State::Configuration;
                // The next game listener starts teleport ids over.
                self.teleports.clear();
                // The closing game listener ignores batch answers and the next one starts over.
                self.chunk_batches.clear();
                return Ok(Some(ClientBoundEvent::PhaseSwitch));
            }
            s2c::game::PlayerChat::PACKET_ID => {
                let chat: s2c::game::PlayerChat = decode(packet, self.threshold)?;
                if let Some(signature) = chat.signature {
                    self.chat.track(signature);
                }
            }
            s2c::game::Ping::PACKET_ID if both_online => {
                let ping: s2c::game::Ping = decode(packet, self.threshold)?;
                self.seq += 1;
                self.pings.push(PingSync::new(
                    ping.id,
                    State::Game,
                    self.stream_phases,
                    self.seq,
                ));
            }
            s2c::game::KeepAlive::PACKET_ID if both_online => {
                let keep_alive: s2c::game::KeepAlive = decode(packet, self.threshold)?;
                self.seq += 1;
                self.keep_alives
                    .push(KeepAliveSync::new(keep_alive.id, State::Game, self.seq));
            }
            s2c::game::ChunkBatchFinished::PACKET_ID if both_online => {
                self.seq += 1;
                self.chunk_batches.push(ChunkBatchSync::new(self.seq));
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
        is_active: bool,
        both_online: bool,
    ) -> anyhow::Result<Option<ServerBoundEvent>> {
        let id = self.ids.read(packet, self.threshold)?;

        match id {
            c2s::configuration::FinishConfiguration::PACKET_ID => {
                *self.state_out_mut(client_id) = State::Game;
                self.chat.restart_client(client_id);
                return Ok(Some(ServerBoundEvent::PhaseAcked));
            }
            c2s::configuration::Pong::PACKET_ID => {
                let pong: c2s::configuration::Pong = decode(packet, self.threshold)?;
                return Ok(self.answer_ping(pong.id, client_id, is_active, both_online));
            }
            c2s::configuration::KeepAlive::PACKET_ID => {
                let keep_alive: c2s::configuration::KeepAlive = decode(packet, self.threshold)?;
                return Ok(self.answer_keep_alive(
                    keep_alive.id,
                    State::Configuration,
                    client_id,
                    is_active,
                    both_online,
                ));
            }
            c2s::configuration::SelectKnownPacks::PACKET_ID => {
                return Ok(self.answer_task(
                    Task::KnownPacks,
                    DONE,
                    packet,
                    client_id,
                    is_active,
                    both_online,
                ));
            }
            c2s::configuration::ResourcePack::PACKET_ID => {
                let response: c2s::configuration::ResourcePack = decode(packet, self.threshold)?;
                return Ok(self.answer_task(
                    Task::ResourcePack(response.id),
                    pack_stage(response.action.0),
                    packet,
                    client_id,
                    is_active,
                    both_online,
                ));
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
            c2s::game::ConfigurationAcknowledged::PACKET_ID => {
                *self.state_out_mut(client_id) = State::Configuration;
                return Ok(Some(ServerBoundEvent::PhaseAcked));
            }
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
                let pong: c2s::game::Pong = decode(packet, self.threshold)?;
                return Ok(self.answer_ping(pong.id, client_id, is_active, both_online));
            }
            c2s::game::KeepAlive::PACKET_ID => {
                let keep_alive: c2s::game::KeepAlive = decode(packet, self.threshold)?;
                return Ok(self.answer_keep_alive(
                    keep_alive.id,
                    State::Game,
                    client_id,
                    is_active,
                    both_online,
                ));
            }
            c2s::game::ChunkBatchReceived::PACKET_ID => {
                return Ok(self.answer_chunk_batch(packet, client_id, is_active, both_online));
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
            c2s::game::ChatCommand::PACKET_ID => {
                if is_active && let Ok(text) = decode::<String>(packet, self.threshold) {
                    config::report_chat(text, true);
                }
            }
            c2s::game::ChatCommandSigned::PACKET_ID => {
                let mut command: c2s::game::ChatCommandSigned = decode(packet, self.threshold)?;
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
            c2s::game::ChatAck::PACKET_ID => {
                let ack: c2s::game::ChatAck = decode(packet, self.threshold)?;
                let relay = is_active && self.phases[client_id] == self.server_phases;
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
        let relay = is_active && self.phases[client_id] == self.server_phases;
        let acked = last_seen.acknowledged.bits();
        let Some((offset, synced)) = self
            .chat
            .update(client_id, last_seen.offset.0, acked, relay)
        else {
            return false;
        };
        // A lagging client can ack the same slots as the server, but not the same messages.
        if offset == last_seen.offset.0
            && synced == acked
            && last_seen.checksum == self.chat.checksum()
        {
            return false;
        }
        *last_seen = c2s::game::LastSeen {
            offset: VarInt(offset),
            acknowledged: Fixed::from_bits(synced),
            checksum: self.chat.checksum(),
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
        if !both_online || self.state_in != State::Game {
            return Ok(None);
        }
        let packet = UncompressedPacket::from_packet(&self.position)?
            .to_raw_packet_compressed(self.threshold)?;
        self.unacked[active.opposite()].push_back(Teleport::Own);
        Ok(Some(ServerBoundEvent::SendToInactive(packet)))
    }

    fn answer_ping(
        &mut self,
        id: i32,
        client_id: ClientId,
        is_active: bool,
        both_online: bool,
    ) -> Option<ServerBoundEvent> {
        // A vanilla client answers a ping after acking every phase switch sent before it.
        let phase = self.phases[client_id];
        let i = self
            .pings
            .iter()
            .position(|s| s.id == id && s.phase == phase && !s.answered(client_id))?;
        let duplicate = self.pings[i].answer(client_id, is_active);
        if !both_online || self.pings[i].answered_by_both() {
            self.pings.remove(i);
        }
        duplicate.then_some(ServerBoundEvent::SkipRelay)
    }

    fn answer_keep_alive(
        &mut self,
        id: i64,
        state: State,
        client_id: ClientId,
        is_active: bool,
        both_online: bool,
    ) -> Option<ServerBoundEvent> {
        let i = self
            .keep_alives
            .iter()
            .position(|s| s.id == id && s.state == state && !s.answered(client_id))?;
        let duplicate = self.keep_alives[i].answer(client_id, is_active);
        if !both_online || self.keep_alives[i].answered_by_both() {
            self.keep_alives.remove(i);
        }
        duplicate.then_some(ServerBoundEvent::SkipRelay)
    }

    fn answer_chunk_batch(
        &mut self,
        packet: &RawPacket,
        client_id: ClientId,
        is_active: bool,
        both_online: bool,
    ) -> Option<ServerBoundEvent> {
        if self.phases[client_id] != self.server_phases {
            // A lagging client answers an earlier game listener's batch.
            return None;
        }
        // Vanilla clients answer every batch in the order they got them.
        let i = self
            .chunk_batches
            .iter()
            .position(|b| !b.answered(client_id))?;
        let duplicate = self.chunk_batches[i].answer(client_id, packet, is_active);
        if !both_online || self.chunk_batches[i].answered_by_both() {
            self.chunk_batches.remove(i);
        }
        duplicate.then_some(ServerBoundEvent::SkipRelay)
    }

    fn track_task(&mut self, task: Task, both_online: bool) {
        if both_online {
            self.seq += 1;
            self.tasks.push(TaskSync::new(task, self.seq));
        }
    }

    fn answer_task(
        &mut self,
        task: Task,
        stage: u8,
        packet: &RawPacket,
        client_id: ClientId,
        is_active: bool,
        both_online: bool,
    ) -> Option<ServerBoundEvent> {
        if self.phases[client_id] != self.server_phases {
            // A lagging client answers an earlier listener's task.
            return None;
        }
        let i = self
            .tasks
            .iter()
            .position(|t| t.task == task && !t.done(client_id))?;
        let duplicate = self.tasks[i].answer(client_id, stage, packet, is_active);
        if self.tasks[i].done_by_both() || !both_online && self.tasks[i].done(client_id) {
            self.tasks.remove(i);
        }
        duplicate.then_some(ServerBoundEvent::SkipRelay)
    }
}

/// How far a resource pack answer gets, by `ServerboundResourcePackPacket.Action` ordinal.
fn pack_stage(action: i32) -> u8 {
    match action {
        3 => ACCEPTED,
        4 => DOWNLOADED,
        _ => DONE,
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
    pub state: State,
    /// The phase switches a client acked when it answers.
    pub phase: u32,
    pub seq: u64,
    answered: PerClient<bool>,
    /// Whether a reply already reached the server.
    sent: bool,
}

impl PingSync {
    pub fn new(id: i32, state: State, phase: u32, seq: u64) -> Self {
        Self {
            id,
            state,
            phase,
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
    /// What the server tracks: its window, then pending messages; `None` is an empty slot.
    tracked: VecDeque<Option<Tracked>>,
    last_pending: Option<Fixed<256>>,
    /// Messages the server's window moved past since its game listener started.
    server_offset: u64,
    /// Messages each client's window moved past since its game listener started.
    offsets: PerClient<u64>,
}

#[derive(Debug)]
struct Tracked {
    signature: Fixed<256>,
    pending: bool,
}

impl Default for ChatSync {
    fn default() -> Self {
        Self {
            tracked: (0..LAST_SEEN).map(|_| None).collect(),
            last_pending: None,
            server_offset: 0,
            offsets: PerClient::splat(0),
        }
    }
}

impl ChatSync {
    /// A new server game listener starts with a fresh validator.
    pub fn restart_server(&mut self) {
        let offsets = self.offsets;
        *self = Self {
            offsets,
            ..Self::default()
        };
    }

    /// A client's new game listener starts with a fresh `LastSeenMessagesTracker`.
    pub fn restart_client(&mut self, client: ClientId) {
        self.offsets[client] = 0;
    }

    /// `sendPlayerChatMessage` tracks every signed message, skipping a repeat of the last one.
    pub fn track(&mut self, signature: Fixed<256>) {
        if self.last_pending != Some(signature) {
            self.tracked.push_back(Some(Tracked {
                signature,
                pending: true,
            }));
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
                Some(tracked) if seen || !tracked.pending => {
                    tracked.pending = false;
                    synced |= 1 << i;
                }
                _ => *slot = None,
            }
        }
        Some((offset, synced))
    }

    /// `LastSeenMessages.computeChecksum` of the messages acked in the server's window.
    pub fn checksum(&self) -> u8 {
        let hash = self
            .tracked
            .iter()
            .take(LAST_SEEN)
            .flatten()
            .fold(1i32, |hash, tracked| {
                // `MessageSignature.checksum` is `Arrays.hashCode` of the signature.
                let signature = tracked.signature.0.iter().fold(1i32, |hash, b| {
                    hash.wrapping_mul(31).wrapping_add(*b as i8 as i32)
                });
                hash.wrapping_mul(31).wrapping_add(signature)
            });
        match hash as u8 {
            0 => 1,
            checksum => checksum,
        }
    }
}

#[derive(Debug)]
pub struct KeepAliveSync {
    pub id: i64,
    pub state: State,
    pub seq: u64,
    answered: PerClient<bool>,
    /// Whether an answer already reached the server.
    sent: bool,
}

impl KeepAliveSync {
    pub fn new(id: i64, state: State, seq: u64) -> Self {
        Self {
            id,
            state,
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

#[derive(Debug)]
pub struct ChunkBatchSync {
    pub seq: u64,
    /// Each client's answer; it carries the client's own `desiredChunksPerTick`.
    answers: PerClient<Option<RawPacket>>,
    /// Whether an answer already reached the server.
    sent: bool,
}

impl ChunkBatchSync {
    pub fn new(seq: u64) -> Self {
        Self {
            seq,
            answers: PerClient::default(),
            sent: false,
        }
    }

    pub fn answered(&self, client: ClientId) -> bool {
        self.answers[client].is_some()
    }

    pub fn answered_by_both(&self) -> bool {
        self.answered(ClientId::Primary) && self.answered(ClientId::Secondary)
    }

    /// Records an answer and returns whether the server already got one.
    pub fn answer(&mut self, client: ClientId, packet: &RawPacket, is_active: bool) -> bool {
        self.answers[client] = Some(packet.clone());
        let duplicate = is_active && self.sent;
        self.sent |= is_active;
        duplicate
    }
}

/// A configuration task request a client must answer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Task {
    KnownPacks,
    ResourcePack(Uuid),
}

#[derive(Debug)]
pub struct TaskSync {
    pub task: Task,
    pub seq: u64,
    /// Each client's answers in the order it sent them, with the stage each one reaches.
    answers: PerClient<Vec<(u8, RawPacket)>>,
    /// The furthest stage the server got.
    sent: u8,
}

impl TaskSync {
    pub fn new(task: Task, seq: u64) -> Self {
        Self {
            task,
            seq,
            answers: PerClient::default(),
            sent: 0,
        }
    }

    pub fn done(&self, client: ClientId) -> bool {
        self.answers[client]
            .last()
            .is_some_and(|(stage, _)| *stage == DONE)
    }

    pub fn done_by_both(&self) -> bool {
        self.done(ClientId::Primary) && self.done(ClientId::Secondary)
    }

    /// Records an answer and returns whether the server already got that far.
    pub fn answer(
        &mut self,
        client: ClientId,
        stage: u8,
        packet: &RawPacket,
        is_active: bool,
    ) -> bool {
        self.answers[client].push((stage, packet.clone()));
        if !is_active {
            return false;
        }
        let duplicate = stage <= self.sent;
        self.sent = self.sent.max(stage);
        duplicate
    }

    /// The client's answers that take the task further than the server got.
    pub fn catch_up(&mut self, client: ClientId) -> Vec<RawPacket> {
        let mut packets = vec![];
        for (stage, packet) in &self.answers[client] {
            if *stage > self.sent {
                self.sent = *stage;
                packets.push(packet.clone());
            }
        }
        packets
    }
}

#[cfg(test)]
mod tests {
    super::super::config_phase_tests!(known_packs, checksum);
}
