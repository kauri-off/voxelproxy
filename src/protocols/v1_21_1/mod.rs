pub mod packets;

use std::collections::VecDeque;

use mc_protocol::{
    packet::{RawPacket, UncompressedPacket},
    varint::VarInt,
};
use uuid::Uuid;

use super::{PacketIdReader, VersionProtocol, decode};
use crate::{
    config,
    events::{ClientId, PerClient},
    protocols::{ClientBoundEvent, ClientDisconnectEvent, ServerBoundEvent},
};
use packets::{c2s, s2c};

/// Vanilla servers never issue negative teleport ids.
const TELEPORT_ID: i32 = -1;
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
                yaw: 0.0,
                pitch: 0.0,
                relative_flags: 0,
            },
            unacked: Default::default(),
            pings: vec![],
            teleports: vec![],
            keep_alives: vec![],
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

        // Vanilla clients answer these in server order; keep-alives only roughly, off the main thread.
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
            c2s::game::ChatCommand::PACKET_ID | c2s::game::Chat::PACKET_ID => {
                if is_active && let Ok(text) = decode::<String>(packet, self.threshold) {
                    let is_command = id == c2s::game::ChatCommand::PACKET_ID;
                    config::report_chat(text, is_command);
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
    use mc_protocol::{packet::PacketId, ser::Serialize};

    use super::*;

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

    fn mirrored(
        threshold: Option<i32>,
        event: Option<ServerBoundEvent>,
    ) -> Option<s2c::game::Position> {
        match event? {
            ServerBoundEvent::SendToInactive(packet) => {
                let id = packet.uncompress(threshold).unwrap().packet_id;
                assert_eq!(id, s2c::game::Position::PACKET_ID);
                Some(super::super::decode(&packet, threshold).unwrap())
            }
            _ => panic!("expected a mirrored position"),
        }
    }

    fn login(data: &mut VersionData, client: ClientId) {
        c2s(data, &c2s::login::LoginAcknowledged {}, client, P, true);
    }

    fn finish_config(data: &mut VersionData, client: ClientId) {
        c2s(
            data,
            &c2s::configuration::FinishConfiguration {},
            client,
            P,
            true,
        );
    }

    fn rot(yaw: f32, pitch: f32) -> c2s::game::Rot {
        c2s::game::Rot {
            yaw,
            pitch,
            ..Default::default()
        }
    }

    fn teleport_ack(id: i32) -> c2s::game::AcceptTeleportation {
        c2s::game::AcceptTeleportation { id: VarInt(id) }
    }

    /// Both clients went through login and configuration; the server is in Game.
    fn in_game(threshold: Option<i32>) -> VersionData {
        let mut data = VersionData::new();
        data.update_threshold(threshold);
        for client in [P, S] {
            login(&mut data, client);
            finish_config(&mut data, client);
        }
        s2c(&mut data, &s2c::configuration::FinishConfiguration {}, true);
        data
    }

    #[test]
    fn phase_switches_and_acks_are_reported() {
        let mut data = VersionData::new();
        login(&mut data, P);
        assert!(matches!(
            c2s(
                &mut data,
                &c2s::configuration::FinishConfiguration {},
                P,
                P,
                true
            ),
            Some(ServerBoundEvent::PhaseAcked)
        ));
        assert!(matches!(
            s2c(&mut data, &s2c::configuration::FinishConfiguration {}, true),
            Some(ClientBoundEvent::PhaseSwitch)
        ));
        assert!(s2c(&mut data, &s2c::game::Ping { id: 1 }, true).is_none());
        assert!(matches!(
            s2c(&mut data, &s2c::game::StartConfiguration {}, true),
            Some(ClientBoundEvent::PhaseSwitch)
        ));
        assert!(matches!(
            c2s(
                &mut data,
                &c2s::game::ConfigurationAcknowledged {},
                P,
                P,
                true
            ),
            Some(ServerBoundEvent::PhaseAcked)
        ));
        assert!(c2s(&mut data, &c2s::login::LoginAcknowledged {}, S, P, true).is_none());
    }

    #[test]
    fn rotation_reaches_inactive_after_late_login_ack() {
        for threshold in [None, Some(0), Some(256)] {
            let mut data = VersionData::new();
            data.update_threshold(threshold);
            login(&mut data, P);
            finish_config(&mut data, P);
            s2c(&mut data, &s2c::configuration::FinishConfiguration {}, true);
            // The secondary is slower and acks login after the server already finished configuration.
            login(&mut data, S);
            finish_config(&mut data, S);

            let event = c2s(&mut data, &rot(90.0, 30.0), P, P, true);
            let pos = mirrored(data.threshold, event).expect("rotation was not mirrored");
            assert_eq!((pos.yaw, pos.pitch), (90.0, 30.0), "{threshold:?}");
        }
    }

    #[test]
    fn movement_is_mirrored_to_inactive() {
        let mut data = in_game(Some(256));

        let pos = c2s::game::Pos {
            x: 1.0,
            y: 2.0,
            z: 3.0,
            ..Default::default()
        };
        let got = mirrored(data.threshold, c2s(&mut data, &pos, P, P, true)).unwrap();
        assert_eq!((got.x, got.y, got.z), (1.0, 2.0, 3.0));

        let got = mirrored(
            data.threshold,
            c2s(&mut data, &rot(45.0, -10.0), P, P, true),
        )
        .unwrap();
        assert_eq!((got.x, got.y, got.z), (1.0, 2.0, 3.0));
        assert_eq!((got.yaw, got.pitch), (45.0, -10.0));

        let pos_rot = c2s::game::PosRot {
            x: 4.0,
            y: 5.0,
            z: 6.0,
            yaw: 180.0,
            pitch: 90.0,
            ..Default::default()
        };
        let got = mirrored(data.threshold, c2s(&mut data, &pos_rot, P, P, true)).unwrap();
        assert_eq!((got.x, got.y, got.z), (4.0, 5.0, 6.0));
        assert_eq!((got.yaw, got.pitch), (180.0, 90.0));
        assert_eq!(got.id.0, TELEPORT_ID);
        assert_eq!(got.relative_flags, 0);
    }

    #[test]
    fn inactive_movement_is_not_mirrored() {
        let mut data = in_game(None);
        assert!(c2s(&mut data, &rot(10.0, 10.0), S, P, true).is_none());
        let got = mirrored(data.threshold, c2s(&mut data, &rot(1.0, 2.0), P, P, true)).unwrap();
        assert_eq!((got.yaw, got.pitch), (1.0, 2.0));
    }

    #[test]
    fn nothing_is_mirrored_during_reconfiguration() {
        let mut data = in_game(None);
        s2c(&mut data, &s2c::game::StartConfiguration {}, true);
        assert!(c2s(&mut data, &rot(1.0, 1.0), P, P, true).is_none());

        for client in [P, S] {
            c2s(
                &mut data,
                &c2s::game::ConfigurationAcknowledged {},
                client,
                P,
                true,
            );
        }
        for client in [P, S] {
            finish_config(&mut data, client);
        }
        s2c(&mut data, &s2c::configuration::FinishConfiguration {}, true);

        let got = mirrored(data.threshold, c2s(&mut data, &rot(5.0, 6.0), P, P, true)).unwrap();
        assert_eq!((got.yaw, got.pitch), (5.0, 6.0));
    }

    #[test]
    fn container_close_is_mirrored_in_game_only() {
        let mut data = in_game(None);
        let close = c2s::game::ContainerClose { container_id: 3 };
        let Some(ServerBoundEvent::SendToInactive(packet)) = c2s(&mut data, &close, P, P, true)
        else {
            panic!("container close was not mirrored");
        };
        let got: s2c::game::ContainerClose = super::super::decode(&packet, None).unwrap();
        assert_eq!(got.container_id, 3);
        assert!(c2s(&mut data, &close, S, P, true).is_none());

        s2c(&mut data, &s2c::game::StartConfiguration {}, true);
        assert!(c2s(&mut data, &close, P, P, true).is_none());
    }

    #[test]
    fn failover_keeps_new_active_client_state() {
        let mut data = in_game(None);
        c2s(&mut data, &rot(1.0, 1.0), P, P, true);
        s2c(&mut data, &s2c::game::StartConfiguration {}, true);
        c2s(
            &mut data,
            &c2s::game::ConfigurationAcknowledged {},
            P,
            P,
            true,
        );

        // The primary drops before the secondary acked the reconfiguration.
        data.handle_client_disconnect(S);
        assert_eq!(data.state_out[S], State::Game);

        let ack = c2s::game::AcceptTeleportation {
            id: VarInt(TELEPORT_ID),
        };
        assert!(matches!(
            c2s(&mut data, &ack, S, S, false),
            Some(ServerBoundEvent::SkipRelay)
        ));
        c2s(
            &mut data,
            &c2s::game::ConfigurationAcknowledged {},
            S,
            S,
            false,
        );
        assert_eq!(data.state_out[S], State::Configuration);
    }

    #[test]
    fn only_pending_own_teleport_acks_are_skipped() {
        let mut data = in_game(None);
        for yaw in [1.0, 2.0] {
            c2s(&mut data, &rot(yaw, 0.0), P, P, true);
        }
        let ours = teleport_ack(TELEPORT_ID);
        assert!(matches!(
            c2s(&mut data, &ours, S, P, true),
            Some(ServerBoundEvent::SkipRelay)
        ));

        // The second ack was still in flight when the primary dropped.
        data.handle_client_disconnect(S);
        assert!(matches!(
            c2s(&mut data, &ours, S, S, false),
            Some(ServerBoundEvent::SkipRelay)
        ));
        assert!(c2s(&mut data, &ours, S, S, false).is_none());
        assert!(c2s(&mut data, &teleport_ack(1), S, S, false).is_none());
    }

    #[test]
    fn acks_are_relayed_for_client_without_own_teleports() {
        let mut data = in_game(None);
        c2s(&mut data, &rot(1.0, 0.0), P, P, true);
        // The inactive secondary left, the primary never got our teleports.
        for both_online in [true, false] {
            let ack = teleport_ack(TELEPORT_ID);
            assert!(c2s(&mut data, &ack, P, P, both_online).is_none());
        }
        assert!(c2s(&mut data, &teleport_ack(5), P, P, false).is_none());
    }

    #[test]
    fn unsent_own_teleports_are_not_counted() {
        let mut data = in_game(None);
        assert!(c2s(&mut data, &rot(1.0, 0.0), P, P, false).is_none());
        s2c(&mut data, &s2c::game::StartConfiguration {}, true);
        assert!(c2s(&mut data, &rot(2.0, 0.0), P, P, true).is_none());
        assert!(data.unacked.iter().all(VecDeque::is_empty));
    }

    #[test]
    fn pong_sent_only_by_new_active_is_relayed_on_failover() {
        let mut data = in_game(Some(256));
        s2c(&mut data, &s2c::game::Ping { id: 7 }, true);
        c2s(&mut data, &c2s::game::Pong { id: 7 }, S, P, true);

        let Some(ClientDisconnectEvent::SendToServer(packets)) = data.handle_client_disconnect(S)
        else {
            panic!("pong was not relayed");
        };
        assert_eq!(packets.len(), 1);
        let pong: c2s::game::Pong = super::super::decode(&packets[0], data.threshold).unwrap();
        assert_eq!(pong.id, 7);
        assert!(data.pings.is_empty());
    }

    #[test]
    fn pong_already_relayed_is_skipped_after_failover() {
        let mut data = in_game(None);
        s2c(&mut data, &s2c::game::Ping { id: 8 }, true);
        c2s(&mut data, &c2s::game::Pong { id: 8 }, P, P, true);

        assert!(data.handle_client_disconnect(S).is_none());
        assert!(matches!(
            c2s(&mut data, &c2s::game::Pong { id: 8 }, S, S, false),
            Some(ServerBoundEvent::SkipRelay)
        ));
        assert!(c2s(&mut data, &c2s::game::Pong { id: 9 }, S, S, false).is_none());
    }

    #[test]
    fn chunk_batch_answers_follow_server_across_failover() {
        let mut data = in_game(None);
        let batch = s2c::game::ChunkBatchFinished {
            batch_size: VarInt(4),
        };
        let answer = |desired_chunks_per_tick| c2s::game::ChunkBatchReceived {
            desired_chunks_per_tick,
        };
        for _ in 0..3 {
            s2c(&mut data, &batch, true);
        }
        assert!(c2s(&mut data, &answer(5.0), P, P, true).is_none());
        for desired in [20.0, 21.0] {
            assert!(c2s(&mut data, &answer(desired), S, P, true).is_none());
        }

        let packets = replayed(data.handle_client_disconnect(S));
        assert_eq!(packets.len(), 1);
        let got: c2s::game::ChunkBatchReceived =
            super::super::decode(&packets[0], data.threshold).unwrap();
        assert_eq!(got.desired_chunks_per_tick, 21.0);
        assert!(c2s(&mut data, &answer(22.0), S, S, false).is_none());
        assert!(data.chunk_batches.is_empty());
    }

    #[test]
    fn configuration_pong_follows_server_across_failover() {
        let mut data = in_game(None);
        s2c(&mut data, &s2c::game::StartConfiguration {}, true);
        for client in [P, S] {
            c2s(
                &mut data,
                &c2s::game::ConfigurationAcknowledged {},
                client,
                P,
                true,
            );
        }
        for id in [1, 2] {
            s2c(&mut data, &s2c::configuration::Ping { id }, true);
        }
        assert!(c2s(&mut data, &c2s::configuration::Pong { id: 1 }, P, P, true).is_none());
        for id in [1, 2] {
            assert!(c2s(&mut data, &c2s::configuration::Pong { id }, S, P, true).is_none());
        }

        let packets = replayed(data.handle_client_disconnect(S));
        assert_eq!(packets.len(), 1);
        let id = packets[0].uncompress(data.threshold).unwrap().packet_id;
        assert_eq!(id, c2s::configuration::Pong::PACKET_ID);
        let pong: c2s::configuration::Pong =
            super::super::decode(&packets[0], data.threshold).unwrap();
        assert_eq!(pong.id, 2);
        assert!(data.pings.is_empty());
    }

    #[test]
    fn pings_are_answered_to_listener_after_phase_switch() {
        let mut data = in_game(None);
        s2c(&mut data, &s2c::game::Ping { id: 1 }, true);
        s2c(&mut data, &s2c::game::StartConfiguration {}, true);
        s2c(&mut data, &s2c::configuration::Ping { id: 2 }, true);
        c2s(&mut data, &c2s::game::Pong { id: 1 }, S, P, true);
        c2s(
            &mut data,
            &c2s::game::ConfigurationAcknowledged {},
            P,
            P,
            true,
        );
        assert_eq!(data.pings.len(), 1);
        c2s(
            &mut data,
            &c2s::game::ConfigurationAcknowledged {},
            S,
            P,
            true,
        );
        c2s(&mut data, &c2s::configuration::Pong { id: 2 }, S, P, true);

        let packets = replayed(data.handle_client_disconnect(S));
        assert_eq!(packets.len(), 1);
        let pong: c2s::configuration::Pong =
            super::super::decode(&packets[0], data.threshold).unwrap();
        assert_eq!(pong.id, 2);
    }

    #[test]
    fn pong_answered_by_both_is_forgotten() {
        let mut data = in_game(None);
        s2c(&mut data, &s2c::game::Ping { id: 1 }, true);
        c2s(&mut data, &c2s::game::Pong { id: 1 }, P, P, true);
        c2s(&mut data, &c2s::game::Pong { id: 1 }, S, P, true);
        assert!(data.pings.is_empty());
        assert!(data.handle_client_disconnect(S).is_none());
    }

    const ACCEPTED_ACTION: i32 = 3;
    const LOADED_ACTION: i32 = 0;
    /// ACCEPTED and DOWNLOADED.
    const IN_FLIGHT: [i32; 2] = [3, 4];

    fn configuring() -> VersionData {
        let mut data = VersionData::new();
        for client in [P, S] {
            login(&mut data, client);
        }
        data
    }

    fn push(data: &mut VersionData, id: u128, both_online: bool) {
        let push = s2c::configuration::ResourcePackPush {
            id: Uuid::from_u128(id),
        };
        s2c(data, &push, both_online);
    }

    fn answer(id: u128, action: i32) -> c2s::configuration::ResourcePack {
        c2s::configuration::ResourcePack {
            id: Uuid::from_u128(id),
            action: VarInt(action),
        }
    }

    fn replayed(event: Option<ClientDisconnectEvent>) -> Vec<RawPacket> {
        match event {
            Some(ClientDisconnectEvent::SendToServer(packets)) => packets,
            None => vec![],
        }
    }

    #[test]
    fn pack_answers_resume_from_what_server_got() {
        let mut data = configuring();
        push(&mut data, 1, true);
        assert!(c2s(&mut data, &answer(1, ACCEPTED_ACTION), P, P, true).is_none());
        for action in IN_FLIGHT {
            assert!(c2s(&mut data, &answer(1, action), S, P, true).is_none());
        }

        let packets = replayed(data.handle_client_disconnect(S));
        let actions: Vec<i32> = packets
            .iter()
            .map(|p| {
                crate::protocols::decode::<c2s::configuration::ResourcePack>(p, data.threshold)
                    .unwrap()
                    .action
                    .0
            })
            .collect();
        assert_eq!(actions, IN_FLIGHT[1..]);
        assert!(c2s(&mut data, &answer(1, LOADED_ACTION), S, S, false).is_none());
        assert!(data.tasks.is_empty());
    }

    #[test]
    fn pack_answers_server_got_are_skipped_after_failover() {
        let mut data = configuring();
        push(&mut data, 1, true);
        for action in IN_FLIGHT {
            assert!(c2s(&mut data, &answer(1, action), P, P, true).is_none());
        }
        assert!(replayed(data.handle_client_disconnect(S)).is_empty());
        for action in IN_FLIGHT {
            assert!(matches!(
                c2s(&mut data, &answer(1, action), S, S, false),
                Some(ServerBoundEvent::SkipRelay)
            ));
        }
        assert!(c2s(&mut data, &answer(1, LOADED_ACTION), S, S, false).is_none());
        assert!(data.tasks.is_empty());
    }

    #[test]
    fn pack_answered_by_both_is_forgotten() {
        let mut data = configuring();
        push(&mut data, 1, true);
        for client in [P, S] {
            c2s(&mut data, &answer(1, LOADED_ACTION), client, P, true);
        }
        assert!(data.tasks.is_empty());
    }

    #[test]
    fn pack_pushed_while_alone_is_not_tracked() {
        let mut data = configuring();
        push(&mut data, 1, false);
        assert!(data.tasks.is_empty());
        assert!(c2s(&mut data, &answer(1, LOADED_ACTION), P, P, false).is_none());
    }

    #[test]
    fn tasks_end_with_configuration_listener() {
        let mut data = configuring();
        push(&mut data, 1, true);
        c2s(&mut data, &answer(1, LOADED_ACTION), P, P, true);
        assert_eq!(data.tasks.len(), 1);
        finish_config(&mut data, P);
        assert!(data.tasks.is_empty());
        assert!(replayed(data.handle_client_disconnect(S)).is_empty());
    }

    #[test]
    fn packs_are_tracked_by_id() {
        let mut data = configuring();
        push(&mut data, 1, true);
        push(&mut data, 2, true);
        c2s(&mut data, &answer(2, LOADED_ACTION), P, P, true);
        c2s(&mut data, &answer(1, LOADED_ACTION), S, P, true);
        assert_eq!(replayed(data.handle_client_disconnect(S)).len(), 1);
        assert!(matches!(
            c2s(&mut data, &answer(2, LOADED_ACTION), S, S, false),
            Some(ServerBoundEvent::SkipRelay)
        ));
        assert!(data.tasks.is_empty());
    }

    #[test]
    fn pack_pushed_again_starts_over() {
        let mut data = configuring();
        push(&mut data, 1, true);
        c2s(&mut data, &answer(1, LOADED_ACTION), P, P, true);
        push(&mut data, 1, false);
        assert!(data.tasks.is_empty());
        assert!(c2s(&mut data, &answer(1, LOADED_ACTION), P, P, false).is_none());
    }

    #[test]
    fn known_packs_answer_is_relayed_once() {
        let mut data = configuring();
        s2c(&mut data, &s2c::configuration::SelectKnownPacks {}, true);
        let select = c2s::configuration::SelectKnownPacks {};
        assert!(c2s(&mut data, &select, P, P, true).is_none());
        assert!(replayed(data.handle_client_disconnect(S)).is_empty());
        assert!(matches!(
            c2s(&mut data, &select, S, S, false),
            Some(ServerBoundEvent::SkipRelay)
        ));
    }

    #[test]
    fn known_packs_answer_of_new_active_is_replayed() {
        let mut data = configuring();
        s2c(&mut data, &s2c::configuration::SelectKnownPacks {}, true);
        c2s(
            &mut data,
            &c2s::configuration::SelectKnownPacks {},
            S,
            P,
            true,
        );
        let packets = replayed(data.handle_client_disconnect(S));
        assert_eq!(packets.len(), 1);
        let id = packets[0].uncompress(data.threshold).unwrap().packet_id;
        assert_eq!(id, c2s::configuration::SelectKnownPacks::PACKET_ID);
        assert!(data.tasks.is_empty());
    }
}
