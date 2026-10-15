pub mod packets;

use std::collections::VecDeque;

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

/// Vanilla servers never issue negative teleport ids.
const TELEPORT_ID: i32 = -1;

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
    /// Teleports each client got and has not acked yet, in the order it got them.
    pub unacked: [VecDeque<Teleport>; 2],
    pub pings: Vec<PingSync>,
    pub teleports: Vec<TeleportSync>,
    /// Counts tracked pings and teleports in server stream order.
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
            // The controller starts after LoginSuccess was relayed to both clients.
            state_in: State::Configuration,
            state_out: [State::Login; 2],
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
        match self.state_out[client_id.index()] {
            State::Login => self.handle_c2s_login(packet, client_id),
            State::Configuration => self.handle_c2s_configuration(packet, client_id),
            State::Game => self.handle_c2s_game(packet, client_id, is_active, both_online),
        }
        .unwrap_or_default()
    }

    fn handle_s2c(&mut self, packet: &RawPacket, both_online: bool) -> Option<ClientBoundEvent> {
        match self.state_in {
            State::Login => Ok(None),
            State::Configuration => self.handle_s2c_configuration(packet),
            State::Game => self.handle_s2c_game(packet, both_online),
        }
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
        both_online: bool,
    ) -> anyhow::Result<Option<ClientBoundEvent>> {
        let id = self.ids.read(packet, self.threshold)?;

        match id {
            s2c::game::StartConfiguration::PACKET_ID => {
                self.state_in = State::Configuration;
                // The next game listener starts teleport ids over.
                self.teleports.clear();
                return Ok(Some(ClientBoundEvent::PhaseSwitch));
            }
            s2c::game::Ping::PACKET_ID if both_online => {
                let ping: s2c::game::Ping = decode(packet, self.threshold)?;
                self.seq += 1;
                self.pings.push(PingSync::new(ping.id, self.seq));
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
                let unacked = &mut self.unacked[client_id.index()];
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
        self.unacked[active.opposite().index()].push_back(Teleport::Own);
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
    answered: [bool; 2],
    /// Whether a reply already reached the server.
    sent: bool,
}

impl PingSync {
    pub fn new(id: i32, seq: u64) -> Self {
        Self {
            id,
            seq,
            answered: [false; 2],
            sent: false,
        }
    }

    pub fn answered(&self, client: ClientId) -> bool {
        self.answered[client.index()]
    }

    pub fn answered_by_both(&self) -> bool {
        self.answered == [true; 2]
    }

    /// Records a reply and returns whether the server already got one.
    pub fn answer(&mut self, client: ClientId, is_active: bool) -> bool {
        self.answered[client.index()] = true;
        let duplicate = is_active && self.sent;
        self.sent |= is_active;
        duplicate
    }
}

#[derive(Debug)]
pub struct TeleportSync {
    pub id: i32,
    pub seq: u64,
    answered: [bool; 2],
    /// Whether an ack already reached the server.
    sent: bool,
}

impl TeleportSync {
    pub fn new(id: i32, seq: u64) -> Self {
        Self {
            id,
            seq,
            answered: [false; 2],
            sent: false,
        }
    }

    pub fn answered(&self, client: ClientId) -> bool {
        self.answered[client.index()]
    }

    pub fn answered_by_both(&self) -> bool {
        self.answered == [true; 2]
    }

    /// Records an ack and returns whether the server already got one.
    pub fn answer(&mut self, client: ClientId, is_active: bool) -> bool {
        self.answered[client.index()] = true;
        let duplicate = is_active && self.sent;
        self.sent |= is_active;
        duplicate
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
        assert_eq!(data.state_out[S.index()], State::Game);

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
        assert_eq!(data.state_out[S.index()], State::Configuration);
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
    fn pong_answered_by_both_is_forgotten() {
        let mut data = in_game(None);
        s2c(&mut data, &s2c::game::Ping { id: 1 }, true);
        c2s(&mut data, &c2s::game::Pong { id: 1 }, P, P, true);
        c2s(&mut data, &c2s::game::Pong { id: 1 }, S, P, true);
        assert!(data.pings.is_empty());
        assert!(data.handle_client_disconnect(S).is_none());
    }
}
