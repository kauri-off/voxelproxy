use std::marker::PhantomData;

use futures::{FutureExt, channel::mpsc::UnboundedReceiver as AppReceiver};
use mc_protocol::{
    packet::{PacketId, RawPacket, UncompressedPacket},
    ser::{Deserialize, Serialize},
};
use tokio::sync::mpsc::{self, Receiver, UnboundedReceiver};

use super::{Controller, ControllerEvent, Outgoing, PEER_QUEUE_BYTES, PeerLink};
use crate::{
    events::{AppEvent, ClientId, EventSink},
    protocols::{Version, VersionProtocol},
};

pub(crate) const P: ClientId = ClientId::Primary;
pub(crate) const S: ClientId = ClientId::Secondary;
/// The teleport id of our mirrored positions.
pub(crate) const OURS: i32 = -1;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct Spot {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub yaw: f32,
    pub pitch: f32,
}

pub(crate) fn spot(x: f64, y: f64, z: f64, yaw: f32, pitch: f32) -> Spot {
    Spot {
        x,
        y,
        z,
        yaw,
        pitch,
    }
}

/// Serverbound packets the tests care about, independent of the version.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum C2s {
    LoginAck,
    FinishConfiguration,
    ConfigurationAck,
    /// The spot is only on the wire for versions with `ACK_MOVES`.
    Teleport(i32, Spot),
    Pos(f64, f64, f64),
    Rot(f32, f32),
    PosRot(Spot),
    Pong(i32),
    ContainerClose(i32),
    Raw(i32),
}

/// Clientbound packets the tests care about, independent of the version.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum S2c {
    Position(i32, Spot),
    Ping(i32),
    ContainerClose(i32),
    StartConfiguration,
    FinishConfiguration,
    Raw(i32, usize),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Phase {
    Login,
    Configuration,
    Game,
}

/// Maps the version-independent packets to one version's wire format.
pub(crate) trait Wire: Default + Into<Version> {
    const CONFIGURATION: bool;
    /// AcceptTeleportation carries the client's position, so the proxy mirrors it as a move.
    const ACK_MOVES: bool = false;

    fn write_c2s(packet: C2s) -> UncompressedPacket;
    fn write_s2c(packet: S2c) -> UncompressedPacket;
    fn read_c2s(phase: Phase, packet: &UncompressedPacket) -> Option<C2s>;
    fn read_s2c(phase: Phase, packet: &UncompressedPacket) -> Option<S2c>;

    fn ack(id: i32, at: Spot) -> C2s {
        C2s::Teleport(id, if Self::ACK_MOVES { at } else { Spot::default() })
    }

    /// What a vanilla client sends back for a packet, see `ClientPacketListener`.
    fn respond(packet: S2c) -> Vec<C2s> {
        match packet {
            S2c::Position(id, at) if Self::ACK_MOVES => vec![Self::ack(id, at)],
            S2c::Position(id, at) => vec![Self::ack(id, at), C2s::PosRot(at)],
            S2c::Ping(id) => vec![C2s::Pong(id)],
            S2c::StartConfiguration => vec![C2s::ConfigurationAck],
            S2c::FinishConfiguration => vec![C2s::FinishConfiguration],
            S2c::ContainerClose(_) | S2c::Raw(..) => vec![],
        }
    }
}

pub(crate) fn pack<T: PacketId + Serialize>(packet: &T) -> UncompressedPacket {
    UncompressedPacket::from_packet(packet).unwrap()
}

pub(crate) fn unpack<T: Deserialize>(packet: &UncompressedPacket) -> T {
    packet.deserialize_payload().unwrap()
}

pub(crate) fn acks(packets: &[C2s]) -> Vec<i32> {
    packets
        .iter()
        .filter_map(|p| match p {
            C2s::Teleport(id, _) => Some(*id),
            _ => None,
        })
        .collect()
}

/// Drives a real `Controller` one event at a time and records what each peer got.
pub(crate) struct Harness<W> {
    controller: Controller,
    clients: [UnboundedReceiver<Outgoing>; 2],
    server: Receiver<RawPacket>,
    app: AppReceiver<AppEvent>,
    client_phase: [Phase; 2],
    server_phase: Phase,
    threshold: Option<i32>,
    stopped: bool,
    wire: PhantomData<W>,
}

impl<W: Wire> Harness<W> {
    pub fn new(threshold: Option<i32>) -> Self {
        let (primary, primary_rx) = PeerLink::new();
        let (secondary, secondary_rx) = PeerLink::new();
        let (remote_tx, server) = mpsc::channel(1024);
        let (_, event_rx) = mpsc::channel(1);
        let (events, app) = EventSink::channel();
        let mut version: Version = W::default().into();
        version.update_threshold(threshold);
        let (client_phase, server_phase) = if W::CONFIGURATION {
            (Phase::Configuration, Phase::Login)
        } else {
            (Phase::Game, Phase::Game)
        };
        Self {
            controller: Controller::new(
                P, primary, secondary, remote_tx, event_rx, version, events,
            ),
            clients: [primary_rx, secondary_rx],
            server,
            app,
            client_phase: [client_phase; 2],
            server_phase,
            threshold,
            stopped: false,
            wire: PhantomData,
        }
    }

    /// Both clients finished login and configuration in lockstep, the primary is active.
    pub fn in_game() -> Self {
        Self::new(Some(0)).join()
    }

    pub fn join(mut self) -> Self {
        if W::CONFIGURATION {
            self.send(P, C2s::LoginAck);
            self.send(S, C2s::LoginAck);
            self.serve(S2c::FinishConfiguration);
            for client in [P, S] {
                assert_eq!(self.obey(client), [S2c::FinishConfiguration]);
            }
            assert_eq!(self.sent(), [C2s::LoginAck, C2s::FinishConfiguration]);
        }
        self
    }

    /// Panics if the controller would wait, so a step never depends on timing.
    pub fn step(&mut self, event: ControllerEvent) {
        assert!(!self.stopped, "the controller already stopped");
        let flow = self
            .controller
            .handle(event)
            .now_or_never()
            .expect("the controller blocked");
        self.stopped = flow.is_break();
    }

    pub fn send(&mut self, client: ClientId, packet: C2s) {
        let raw = self.encode(W::write_c2s(packet));
        self.step(ControllerEvent::ClientData(client, raw));
    }

    pub fn send_all(&mut self, client: ClientId, packets: impl IntoIterator<Item = C2s>) {
        for packet in packets {
            self.send(client, packet);
        }
    }

    pub fn serve(&mut self, packet: S2c) {
        let raw = self.encode(W::write_s2c(packet));
        self.step(ControllerEvent::ServerData(raw));
    }

    pub fn leave(&mut self, client: ClientId) {
        self.step(ControllerEvent::ClientDisconnected(client));
    }

    pub fn stopped(&self) -> bool {
        self.stopped
    }

    /// Takes everything queued for a client so far.
    pub fn received(&mut self, client: ClientId) -> Vec<S2c> {
        let mut got = vec![];
        while let Ok((raw, _permit)) = self.clients[client.index()].try_recv() {
            let packet = raw.uncompress(self.threshold).unwrap();
            let phase = &mut self.client_phase[client.index()];
            let packet = W::read_s2c(*phase, &packet)
                .unwrap_or(S2c::Raw(packet.packet_id, packet.payload.len()));
            match packet {
                S2c::FinishConfiguration => *phase = Phase::Game,
                S2c::StartConfiguration => *phase = Phase::Configuration,
                _ => {}
            }
            got.push(packet);
        }
        got
    }

    /// Takes everything the server got so far.
    pub fn sent(&mut self) -> Vec<C2s> {
        let mut got = vec![];
        while let Ok(raw) = self.server.try_recv() {
            let packet = raw.uncompress(self.threshold).unwrap();
            let packet =
                W::read_c2s(self.server_phase, &packet).unwrap_or(C2s::Raw(packet.packet_id));
            match packet {
                C2s::LoginAck | C2s::ConfigurationAck => self.server_phase = Phase::Configuration,
                C2s::FinishConfiguration => self.server_phase = Phase::Game,
                _ => {}
            }
            got.push(packet);
        }
        got
    }

    /// The client handles everything it got like vanilla, including what its answers release.
    pub fn obey(&mut self, client: ClientId) -> Vec<S2c> {
        let mut handled = vec![];
        loop {
            let got = self.received(client);
            if got.is_empty() {
                return handled;
            }
            for packet in got {
                self.send_all(client, W::respond(packet));
                handled.push(packet);
            }
        }
    }

    /// Clients the controller reported as gone since the last call.
    pub fn offline(&mut self) -> Vec<ClientId> {
        let mut gone = vec![];
        while let Ok(event) = self.app.try_recv() {
            if let AppEvent::ClientStatus {
                which,
                online: false,
            } = event
            {
                gone.push(which);
            }
        }
        gone
    }

    fn encode(&self, packet: UncompressedPacket) -> RawPacket {
        packet.to_raw_packet_compressed(self.threshold).unwrap()
    }
}

fn ours(at: Spot) -> S2c {
    S2c::Position(OURS, at)
}

pub(crate) fn active_movement_is_mirrored<W: Wire>() {
    let mut h = Harness::<W>::in_game();

    h.send(P, C2s::Pos(1.0, 2.0, 3.0));
    assert_eq!(h.sent(), [C2s::Pos(1.0, 2.0, 3.0)]);
    assert_eq!(h.received(S), [ours(spot(1.0, 2.0, 3.0, 0.0, 0.0))]);

    h.send(P, C2s::Rot(45.0, -10.0));
    assert_eq!(h.sent(), [C2s::Rot(45.0, -10.0)]);
    assert_eq!(h.received(S), [ours(spot(1.0, 2.0, 3.0, 45.0, -10.0))]);

    let at = spot(4.0, 5.0, 6.0, 180.0, 90.0);
    h.send(P, C2s::PosRot(at));
    assert_eq!(h.sent(), [C2s::PosRot(at)]);
    assert_eq!(h.received(S), [ours(at)]);
    assert!(h.received(P).is_empty());
}

pub(crate) fn inactive_client_never_reaches_server<W: Wire>() {
    let mut h = Harness::<W>::in_game();
    let at = spot(1.0, 2.0, 3.0, 4.0, 5.0);
    h.send(P, C2s::PosRot(at));
    h.sent();

    assert_eq!(h.obey(S), [ours(at)]);
    h.send_all(
        S,
        [
            C2s::Pos(7.0, 8.0, 9.0),
            C2s::Rot(1.0, 1.0),
            C2s::PosRot(spot(0.0, 1.0, 0.0, 1.0, 0.0)),
            C2s::ContainerClose(2),
            C2s::Pong(3),
            C2s::Teleport(5, Spot::default()),
            C2s::Raw(1000),
        ],
    );
    assert!(h.sent().is_empty());
    assert!(h.received(P).is_empty());
}

pub(crate) fn active_teleport_ack_is_mirrored<W: Wire>() {
    let mut h = Harness::<W>::in_game();
    let at = spot(10.0, 64.0, -5.0, 90.0, 15.0);
    h.serve(S2c::Position(5, at));

    assert_eq!(h.obey(P), [S2c::Position(5, at)]);
    assert_eq!(h.sent(), W::respond(S2c::Position(5, at)));
    assert_eq!(h.obey(S), [S2c::Position(5, at), ours(at)]);
    assert!(h.sent().is_empty());
}

pub(crate) fn own_acks_in_flight_are_dropped_after_failover<W: Wire>() {
    let mut h = Harness::<W>::in_game();
    for yaw in [1.0, 2.0, 3.0] {
        h.send(P, C2s::Rot(yaw, 0.0));
    }
    h.sent();
    let mirrored = h.received(S);
    assert_eq!(mirrored.len(), 3);

    h.send_all(S, W::respond(mirrored[0]));
    h.leave(P);
    assert!(h.sent().is_empty());
    for packet in &mirrored[1..] {
        h.send_all(S, W::respond(*packet));
    }
    assert_eq!(acks(&h.sent()), Vec::<i32>::new());

    let at = spot(3.0, 70.0, 3.0, 0.0, 0.0);
    h.serve(S2c::Position(OURS, at));
    assert_eq!(h.obey(S), [S2c::Position(OURS, at)]);
    assert_eq!(h.sent(), W::respond(S2c::Position(OURS, at)));
}

pub(crate) fn server_teleports_after_failover_are_acked_once<W: Wire>() {
    let mut h = Harness::<W>::in_game();
    h.send(P, C2s::Rot(1.0, 0.0));
    h.send(P, C2s::Rot(2.0, 0.0));
    h.sent();
    let mirrored = h.received(S);

    h.send_all(S, W::respond(mirrored[0]));
    h.leave(P);
    h.serve(S2c::Position(1000, spot(1.0, 2.0, 3.0, 0.0, 0.0)));
    h.serve(S2c::Position(OURS, spot(4.0, 5.0, 6.0, 0.0, 0.0)));
    h.send_all(S, W::respond(mirrored[1]));
    h.obey(S);
    assert_eq!(acks(&h.sent()), [1000, OURS]);
}

pub(crate) fn server_teleports_after_inactive_left_are_acked_once<W: Wire>() {
    let mut h = Harness::<W>::in_game();
    h.send(P, C2s::Rot(1.0, 0.0));
    h.send(P, C2s::Rot(2.0, 0.0));
    h.sent();

    h.leave(S);
    h.serve(S2c::Position(1000, spot(1.0, 2.0, 3.0, 0.0, 0.0)));
    h.serve(S2c::Position(OURS, spot(4.0, 5.0, 6.0, 0.0, 0.0)));
    h.obey(P);
    assert_eq!(acks(&h.sent()), [1000, OURS]);
}

pub(crate) fn server_teleport_with_own_id_is_acked_once<W: Wire>() {
    let mut h = Harness::<W>::in_game();
    let at = spot(1.0, 2.0, 3.0, 0.0, 0.0);
    h.serve(S2c::Position(OURS, at));
    h.obey(P);
    assert_eq!(acks(&h.sent()), [OURS]);

    let got = h.received(S);
    assert_eq!(got, [S2c::Position(OURS, at), ours(at)]);
    h.send_all(S, W::respond(got[0]));
    h.leave(P);
    h.send_all(S, W::respond(got[1]));
    assert_eq!(acks(&h.sent()), Vec::<i32>::new());
}

/// The server sends a teleport with our id, then the inactive client gets one of ours.
fn server_teleport_then_ours<W: Wire>(h: &mut Harness<W>, at: Spot) -> Vec<S2c> {
    h.serve(S2c::Position(OURS, at));
    h.send(P, C2s::Rot(1.0, 1.0));
    h.sent();
    let got = h.received(S);
    assert_eq!(got[0], S2c::Position(OURS, at));
    assert!(got.len() > 1);
    got
}

pub(crate) fn inactive_ack_of_server_teleport_with_own_id_is_not_ours<W: Wire>() {
    let mut h = Harness::<W>::in_game();
    let at = spot(1.0, 2.0, 3.0, 0.0, 0.0);
    let got = server_teleport_then_ours(&mut h, at);
    h.received(P);
    h.send_all(S, W::respond(got[0]));
    assert!(h.sent().is_empty());

    h.leave(P);
    assert_eq!(h.sent(), [W::ack(OURS, at)]);
    for packet in &got[1..] {
        h.send_all(S, W::respond(*packet));
    }
    assert_eq!(acks(&h.sent()), Vec::<i32>::new());
}

pub(crate) fn failover_replays_ack_of_server_teleport_with_own_id<W: Wire>() {
    let mut h = Harness::<W>::in_game();
    let at = spot(1.0, 2.0, 3.0, 0.0, 0.0);
    let got = server_teleport_then_ours(&mut h, at);
    h.received(P);
    for packet in got {
        h.send_all(S, W::respond(packet));
    }
    assert!(h.sent().is_empty());

    h.leave(P);
    assert_eq!(h.sent(), [W::ack(OURS, at)]);
}

pub(crate) fn own_ack_after_reconfiguration_is_dropped<W: Wire>() {
    let mut h = Harness::<W>::in_game();
    let at = spot(1.0, 2.0, 3.0, 0.0, 0.0);
    let got = server_teleport_then_ours(&mut h, at);
    h.obey(P);
    let got = [got, h.received(S)].concat();
    h.send_all(S, W::respond(got[0]));
    h.serve(S2c::StartConfiguration);
    h.sent();

    h.leave(P);
    for packet in &got[1..] {
        h.send_all(S, W::respond(*packet));
    }
    assert_eq!(acks(&h.sent()), Vec::<i32>::new());
}

pub(crate) fn container_close_is_mirrored<W: Wire>() {
    let mut h = Harness::<W>::in_game();
    h.send(P, C2s::ContainerClose(3));
    assert_eq!(h.sent(), [C2s::ContainerClose(3)]);
    assert_eq!(h.received(S), [S2c::ContainerClose(3)]);

    h.send(S, C2s::ContainerClose(4));
    assert!(h.sent().is_empty());
    assert!(h.received(P).is_empty());
}

pub(crate) fn nothing_is_mirrored_during_reconfiguration<W: Wire>() {
    let mut h = Harness::<W>::in_game();
    h.serve(S2c::StartConfiguration);
    assert_eq!(h.received(P), [S2c::StartConfiguration]);
    assert_eq!(h.received(S), [S2c::StartConfiguration]);

    h.send(P, C2s::Rot(1.0, 1.0));
    h.send(P, C2s::ContainerClose(3));
    h.send(P, C2s::ConfigurationAck);
    assert_eq!(
        h.sent(),
        [
            C2s::Rot(1.0, 1.0),
            C2s::ContainerClose(3),
            C2s::ConfigurationAck
        ]
    );
    h.send(S, C2s::ConfigurationAck);
    assert!(h.received(S).is_empty());

    h.serve(S2c::FinishConfiguration);
    for client in [P, S] {
        assert_eq!(h.obey(client), [S2c::FinishConfiguration]);
    }
    assert_eq!(h.sent(), [C2s::FinishConfiguration]);
    h.send(P, C2s::Rot(5.0, 6.0));
    assert_eq!(h.received(S), [ours(spot(0.0, 0.0, 0.0, 5.0, 6.0))]);
}

pub(crate) fn start_configuration_holds_packets_until_ack<W: Wire>() {
    let mut h = Harness::<W>::in_game();
    h.serve(S2c::StartConfiguration);
    h.serve(S2c::Raw(1001, 0));
    h.serve(S2c::Raw(1002, 0));
    assert_eq!(h.received(P), [S2c::StartConfiguration]);
    assert_eq!(h.received(S), [S2c::StartConfiguration]);

    h.send(P, C2s::ConfigurationAck);
    assert_eq!(h.received(P), [S2c::Raw(1001, 0), S2c::Raw(1002, 0)]);
    assert!(h.received(S).is_empty());

    h.serve(S2c::Raw(1003, 0));
    assert_eq!(h.received(P), [S2c::Raw(1003, 0)]);
    assert!(h.received(S).is_empty());

    h.send(S, C2s::ConfigurationAck);
    assert_eq!(
        h.received(S),
        [S2c::Raw(1001, 0), S2c::Raw(1002, 0), S2c::Raw(1003, 0)]
    );
    assert_eq!(h.sent(), [C2s::ConfigurationAck]);
}

pub(crate) fn finish_configuration_holds_packets_until_ack<W: Wire>() {
    let mut h = Harness::<W>::new(Some(0));
    h.send(P, C2s::LoginAck);
    h.send(S, C2s::LoginAck);
    h.serve(S2c::FinishConfiguration);
    h.serve(S2c::Ping(1));
    h.serve(S2c::Raw(1001, 4));
    assert_eq!(h.received(P), [S2c::FinishConfiguration]);
    assert_eq!(h.received(S), [S2c::FinishConfiguration]);

    h.send(S, C2s::FinishConfiguration);
    assert_eq!(h.received(S), [S2c::Ping(1), S2c::Raw(1001, 4)]);
    assert!(h.received(P).is_empty());

    h.send(P, C2s::FinishConfiguration);
    assert_eq!(h.received(P), [S2c::Ping(1), S2c::Raw(1001, 4)]);
    assert_eq!(h.sent(), [C2s::LoginAck, C2s::FinishConfiguration]);
}

pub(crate) fn held_packets_stop_at_next_phase_switch<W: Wire>() {
    let mut h = Harness::<W>::in_game();
    h.serve(S2c::StartConfiguration);
    h.serve(S2c::Raw(1001, 0));
    h.serve(S2c::FinishConfiguration);
    h.serve(S2c::Raw(1002, 0));

    assert_eq!(
        h.obey(P),
        [
            S2c::StartConfiguration,
            S2c::Raw(1001, 0),
            S2c::FinishConfiguration,
            S2c::Raw(1002, 0)
        ]
    );
    assert_eq!(h.received(S), [S2c::StartConfiguration]);
    h.send(S, C2s::ConfigurationAck);
    assert_eq!(h.received(S), [S2c::Raw(1001, 0), S2c::FinishConfiguration]);
    h.send(S, C2s::FinishConfiguration);
    assert_eq!(h.received(S), [S2c::Raw(1002, 0)]);
    assert_eq!(h.sent(), [C2s::ConfigurationAck, C2s::FinishConfiguration]);
}

pub(crate) fn failover_keeps_packets_held_for_new_active<W: Wire>() {
    let mut h = Harness::<W>::in_game();
    h.serve(S2c::StartConfiguration);
    h.obey(P);
    h.serve(S2c::Raw(1001, 0));
    assert_eq!(h.received(P), [S2c::Raw(1001, 0)]);
    assert_eq!(h.received(S), [S2c::StartConfiguration]);

    h.leave(P);
    h.serve(S2c::Raw(1002, 0));
    assert!(h.received(S).is_empty());
    h.send(S, C2s::ConfigurationAck);
    assert_eq!(h.received(S), [S2c::Raw(1001, 0), S2c::Raw(1002, 0)]);
}

pub(crate) fn failover_during_reconfiguration_keeps_server_phase<W: Wire>() {
    let mut h = Harness::<W>::in_game();
    h.serve(S2c::StartConfiguration);
    h.obey(P);
    assert_eq!(h.sent(), [C2s::ConfigurationAck]);

    h.leave(P);
    h.send(S, C2s::Rot(1.0, 1.0));
    h.obey(S);
    assert_eq!(h.sent(), []);
}

pub(crate) fn active_pong_reaches_server_once<W: Wire>() {
    let mut h = Harness::<W>::in_game();
    h.serve(S2c::Ping(7));
    assert_eq!(h.received(P), [S2c::Ping(7)]);
    assert_eq!(h.received(S), [S2c::Ping(7)]);
    h.send(P, C2s::Pong(7));
    h.send(S, C2s::Pong(7));
    assert_eq!(h.sent(), [C2s::Pong(7)]);

    h.serve(S2c::Ping(8));
    h.send(S, C2s::Pong(8));
    assert!(h.sent().is_empty());
    h.send(P, C2s::Pong(8));
    assert_eq!(h.sent(), [C2s::Pong(8)]);
}

pub(crate) fn failover_sends_pongs_only_new_active_answered<W: Wire>() {
    let mut h = Harness::<W>::in_game();
    for id in [7, 8, 9] {
        h.serve(S2c::Ping(id));
    }
    h.send(P, C2s::Pong(7));
    h.send_all(S, [C2s::Pong(7), C2s::Pong(8)]);
    assert_eq!(h.sent(), [C2s::Pong(7)]);

    h.leave(P);
    assert_eq!(h.sent(), [C2s::Pong(8)]);
    h.send(S, C2s::Pong(9));
    assert_eq!(h.sent(), [C2s::Pong(9)]);
}

pub(crate) fn forwarded_pongs_are_dropped_after_failover<W: Wire>() {
    let mut h = Harness::<W>::in_game();
    h.serve(S2c::Ping(7));
    h.serve(S2c::Ping(8));
    h.send_all(P, [C2s::Pong(7), C2s::Pong(8)]);
    assert_eq!(h.sent(), [C2s::Pong(7), C2s::Pong(8)]);

    h.leave(P);
    assert!(h.sent().is_empty());
    h.send_all(S, [C2s::Pong(7), C2s::Pong(8)]);
    assert!(h.sent().is_empty());

    h.serve(S2c::Ping(9));
    h.send(S, C2s::Pong(9));
    assert_eq!(h.sent(), [C2s::Pong(9)]);
}

pub(crate) fn active_pong_reaches_server_after_inactive_left<W: Wire>() {
    let mut h = Harness::<W>::in_game();
    h.serve(S2c::Ping(7));
    h.send(S, C2s::Pong(7));
    h.leave(S);
    h.send(P, C2s::Pong(7));
    assert_eq!(h.sent(), [C2s::Pong(7)]);
}

pub(crate) fn inactive_teleport_ack_reaches_server_on_failover<W: Wire>() {
    let mut h = Harness::<W>::in_game();
    let at = spot(1.0, 2.0, 3.0, 0.0, 0.0);
    h.serve(S2c::Position(5, at));
    h.obey(S);
    assert!(h.sent().is_empty());
    assert_eq!(h.received(P), [S2c::Position(5, at)]);

    h.leave(P);
    h.send(S, C2s::Rot(1.0, 1.0));
    assert_eq!(acks(&h.sent()), [5]);
}

pub(crate) fn teleport_ack_is_not_repeated_on_failover<W: Wire>() {
    let mut h = Harness::<W>::in_game();
    let at = spot(1.0, 2.0, 3.0, 0.0, 0.0);
    h.serve(S2c::Position(5, at));
    h.obey(P);
    h.obey(S);
    assert_eq!(acks(&h.sent()), [5]);

    h.leave(P);
    h.send(S, C2s::Rot(1.0, 1.0));
    assert_eq!(h.sent(), [C2s::Rot(1.0, 1.0)]);
}

pub(crate) fn forwarded_teleport_acks_are_dropped_after_failover<W: Wire>() {
    let mut h = Harness::<W>::in_game();
    let at = spot(1.0, 2.0, 3.0, 0.0, 0.0);
    h.serve(S2c::Position(5, at));
    h.obey(P);
    assert_eq!(acks(&h.sent()), [5]);

    h.leave(P);
    assert!(h.sent().is_empty());
    h.obey(S);
    assert_eq!(acks(&h.sent()), Vec::<i32>::new());

    let next = spot(4.0, 5.0, 6.0, 0.0, 0.0);
    h.serve(S2c::Position(6, next));
    h.obey(S);
    assert_eq!(acks(&h.sent()), [6]);
}

pub(crate) fn failover_sends_acks_only_new_active_answered<W: Wire>() {
    let mut h = Harness::<W>::in_game();
    let teleports = [5, 6, 7].map(|id| S2c::Position(id, spot(id as f64, 64.0, 0.0, 0.0, 0.0)));
    for packet in teleports {
        h.serve(packet);
    }
    h.received(P);
    h.received(S);
    h.send_all(P, W::respond(teleports[0]));
    h.send_all(S, W::respond(teleports[0]));
    h.send_all(S, W::respond(teleports[1]));
    assert_eq!(acks(&h.sent()), [5]);

    h.leave(P);
    assert_eq!(h.sent(), [W::ack(6, spot(6.0, 64.0, 0.0, 0.0, 0.0))]);
    h.send_all(S, W::respond(teleports[2]));
    assert_eq!(h.sent(), W::respond(teleports[2]));
}

pub(crate) fn failover_replies_follow_server_order<W: Wire>() {
    let mut h = Harness::<W>::in_game();
    let at = spot(1.0, 2.0, 3.0, 0.0, 0.0);
    h.serve(S2c::Ping(1));
    h.serve(S2c::Position(5, at));
    h.serve(S2c::Ping(2));
    h.received(P);
    h.obey(S);
    assert!(h.sent().is_empty());

    h.leave(P);
    assert_eq!(h.sent(), [C2s::Pong(1), W::ack(5, at), C2s::Pong(2)]);
}

/// Just over half the budget, so the second one overflows it.
const BIG: usize = PEER_QUEUE_BYTES / 2 + 1;

fn big() -> RawPacket {
    let mut data = vec![0; BIG];
    data[..2].copy_from_slice(&[0xe8, 0x07]);
    RawPacket::new(data)
}

fn stalled_inactive_is_dropped<W: Wire>() {
    let mut h = Harness::<W>::new(None).join();
    for _ in 0..2 {
        h.step(ControllerEvent::ServerData(big()));
        assert_eq!(h.received(P), [S2c::Raw(1000, BIG - 2)]);
    }
    assert_eq!(h.offline(), [S]);
    assert_eq!(h.received(S), [S2c::Raw(1000, BIG - 2)]);

    h.send(P, C2s::Rot(1.0, 1.0));
    assert_eq!(h.sent(), [C2s::Rot(1.0, 1.0)]);
    assert!(h.received(S).is_empty());
    assert!(!h.stopped());
}

fn stalled_held_inactive_is_dropped<W: Wire>() {
    let mut h = Harness::<W>::new(None).join();
    h.serve(S2c::StartConfiguration);
    h.obey(P);
    for _ in 0..2 {
        h.step(ControllerEvent::ServerData(big()));
        assert_eq!(h.received(P), [S2c::Raw(1000, BIG - 2)]);
    }
    assert_eq!(h.offline(), [S]);
    assert_eq!(h.received(S), [S2c::StartConfiguration]);
    assert!(!h.stopped());
}

#[test]
fn stalled_inactive_is_dropped_v1_16_5() {
    stalled_inactive_is_dropped::<crate::protocols::v1_16_5::VersionData>();
}

#[test]
fn stalled_inactive_is_dropped_v26_3() {
    stalled_inactive_is_dropped::<crate::protocols::v26_3::VersionData>();
}

#[test]
fn stalled_held_inactive_is_dropped_v26_3() {
    stalled_held_inactive_is_dropped::<crate::protocols::v26_3::VersionData>();
}

#[test]
fn last_client_leaving_stops_controller() {
    let mut h = Harness::<crate::protocols::v26_3::VersionData>::in_game();
    h.leave(S);
    assert!(!h.stopped());
    h.leave(P);
    assert!(h.stopped());
}

/// Runs the version-independent scenarios against the `VersionData` in scope.
macro_rules! controller_tests {
    (@run $($(#[$attr:meta])* $name:ident)*) => {
        mod controller {
            use super::VersionData;
            $(
                #[test]
                $(#[$attr])*
                fn $name() {
                    $crate::controller::tests::$name::<VersionData>();
                }
            )*
        }
    };
    (configuration) => {
        $crate::controller::tests::controller_tests!(@common
            nothing_is_mirrored_during_reconfiguration
            start_configuration_holds_packets_until_ack
            finish_configuration_holds_packets_until_ack
            held_packets_stop_at_next_phase_switch
            failover_keeps_packets_held_for_new_active
            #[ignore = "bug: after failover mid-reconfiguration the new active client's game packets reach a server already in configuration"]
            failover_during_reconfiguration_keeps_server_phase
            own_ack_after_reconfiguration_is_dropped
        );
    };
    () => {
        $crate::controller::tests::controller_tests!(@common);
    };
    (@common $($extra:tt)*) => {
        $crate::controller::tests::controller_tests!(@run
            active_movement_is_mirrored
            inactive_client_never_reaches_server
            active_teleport_ack_is_mirrored
            own_acks_in_flight_are_dropped_after_failover
            server_teleports_after_failover_are_acked_once
            server_teleports_after_inactive_left_are_acked_once
            server_teleport_with_own_id_is_acked_once
            inactive_ack_of_server_teleport_with_own_id_is_not_ours
            failover_replays_ack_of_server_teleport_with_own_id
            container_close_is_mirrored
            active_pong_reaches_server_once
            failover_sends_pongs_only_new_active_answered
            forwarded_pongs_are_dropped_after_failover
            active_pong_reaches_server_after_inactive_left
            inactive_teleport_ack_reaches_server_on_failover
            teleport_ack_is_not_repeated_on_failover
            forwarded_teleport_acks_are_dropped_after_failover
            failover_sends_acks_only_new_active_answered
            failover_replies_follow_server_order
            $($extra)*
        );
    };
}
pub(crate) use controller_tests;
