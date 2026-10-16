use std::marker::PhantomData;

use futures::{FutureExt, channel::mpsc::UnboundedReceiver as AppReceiver};
use mc_protocol::{
    packet::{PacketId, RawPacket, UncompressedPacket},
    ser::{Deserialize, Serialize},
};
use tokio::sync::mpsc::{self, Receiver, UnboundedReceiver};

use super::{Controller, ControllerEvent, Outgoing, PEER_QUEUE_BYTES, PeerLink};
use crate::{
    events::{AppEvent, ClientId, EventSink, PerClient},
    protocols::{Version, VersionProtocol},
};

pub(crate) const P: ClientId = ClientId::Primary;
pub(crate) const S: ClientId = ClientId::Secondary;
/// The teleport id of our mirrored positions.
pub(crate) const OURS: i32 = -1;
/// What a fresh `ChunkBatchSizeCalculator` asks for.
pub(crate) const CHUNKS_PER_TICK: f32 = 3.5;

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
    KeepAlive(i64),
    ContainerClose(i32),
    KnownPacks,
    AcceptCodeOfConduct,
    /// The pack id is only on the wire for versions with `PACK_IDS`.
    ResourcePack(u8, Pack),
    ChunkBatchReceived(f32),
    ChatAck(i32),
    /// With the client's `Instant.now()` in epoch millis, as `writeInstant` puts it on the wire.
    Chat(Seen, i64),
    /// A command with signable arguments, the only kind carrying acks since 1.20.5.
    SignedCommand(Seen, i64),
    Raw(i32),
}

/// A `LastSeenMessages.Update`; the checksum is only on the wire for versions with `CHECKSUM`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Seen {
    pub offset: i32,
    pub acked: u32,
    pub checksum: u8,
}

/// Resource pack answers, numbered like `ServerboundResourcePackPacket.Action`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Pack {
    Loaded = 0,
    Declined = 1,
    FailedDownload = 2,
    Accepted = 3,
    Downloaded = 4,
}

impl Pack {
    pub fn from_action(action: i32) -> Self {
        match action {
            0 => Pack::Loaded,
            1 => Pack::Declined,
            2 => Pack::FailedDownload,
            3 => Pack::Accepted,
            4 => Pack::Downloaded,
            _ => panic!("unexpected resource pack action {action}"),
        }
    }
}

/// Clientbound packets the tests care about, independent of the version.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum S2c {
    Position(i32, Spot),
    Ping(i32),
    KeepAlive(i64),
    ContainerClose(i32),
    StartConfiguration,
    FinishConfiguration,
    SelectKnownPacks,
    CodeOfConduct,
    ResourcePackPush(u8),
    ChunkBatchFinished(i32),
    /// A player message, with the byte its signature is filled with if it is signed.
    PlayerChat(Option<u8>),
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
    /// Resource packs have UUIDs and a DOWNLOADED answer, since 1.20.3.
    const PACK_IDS: bool = true;
    /// Last seen updates carry a checksum, since 1.21.5.
    const CHECKSUM: bool = false;
    /// The server kicks for chat older than the last, before 1.20.3.
    const CHAT_ORDER: bool = false;

    fn write_c2s(phase: Phase, packet: C2s) -> UncompressedPacket;
    fn write_s2c(phase: Phase, packet: S2c) -> UncompressedPacket;
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
            S2c::KeepAlive(id) => vec![C2s::KeepAlive(id)],
            S2c::StartConfiguration => vec![C2s::ConfigurationAck],
            S2c::FinishConfiguration => vec![C2s::FinishConfiguration],
            S2c::SelectKnownPacks => vec![C2s::KnownPacks],
            S2c::CodeOfConduct => vec![C2s::AcceptCodeOfConduct],
            S2c::ResourcePackPush(id) if Self::PACK_IDS => {
                [Pack::Accepted, Pack::Downloaded, Pack::Loaded]
                    .map(|answer| C2s::ResourcePack(id, answer))
                    .to_vec()
            }
            S2c::ResourcePackPush(id) => vec![
                C2s::ResourcePack(id, Pack::Accepted),
                C2s::ResourcePack(id, Pack::Loaded),
            ],
            S2c::ChunkBatchFinished(_) => vec![C2s::ChunkBatchReceived(CHUNKS_PER_TICK)],
            S2c::ContainerClose(_) | S2c::PlayerChat(_) | S2c::Raw(..) => vec![],
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
    clients: PerClient<UnboundedReceiver<Outgoing>>,
    server: Receiver<RawPacket>,
    app: AppReceiver<AppEvent>,
    client_phase: PerClient<Phase>,
    client_out: PerClient<Phase>,
    server_phase: Phase,
    server_out: Phase,
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
        let (client_out, server_out) = (server_phase, client_phase);
        Self {
            controller: Controller::new(
                P, primary, secondary, remote_tx, event_rx, version, events,
            ),
            clients: PerClient::new(primary_rx, secondary_rx),
            server,
            app,
            client_phase: PerClient::splat(client_phase),
            client_out: PerClient::splat(client_out),
            server_phase,
            server_out,
            threshold,
            stopped: false,
            wire: PhantomData,
        }
    }

    /// Both clients finished login and configuration in lockstep, the primary is active.
    pub fn in_game() -> Self {
        Self::new(Some(0)).join()
    }

    /// Both clients acked login, the server runs its configuration tasks.
    pub fn in_configuration() -> Self {
        let mut h = Self::new(Some(0));
        h.send(P, C2s::LoginAck);
        h.send(S, C2s::LoginAck);
        assert_eq!(h.sent(), [C2s::LoginAck]);
        h
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
        let phase = &mut self.client_out[client];
        let raw = W::write_c2s(*phase, packet);
        match packet {
            C2s::LoginAck | C2s::ConfigurationAck => *phase = Phase::Configuration,
            C2s::FinishConfiguration => *phase = Phase::Game,
            _ => {}
        }
        let raw = self.encode(raw);
        self.step(ControllerEvent::ClientData(client, raw));
    }

    pub fn send_all(&mut self, client: ClientId, packets: impl IntoIterator<Item = C2s>) {
        for packet in packets {
            self.send(client, packet);
        }
    }

    pub fn serve(&mut self, packet: S2c) {
        let raw = self.encode(W::write_s2c(self.server_out, packet));
        match packet {
            S2c::StartConfiguration => self.server_out = Phase::Configuration,
            S2c::FinishConfiguration => self.server_out = Phase::Game,
            _ => {}
        }
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
        while let Ok((raw, _permit)) = self.clients[client].try_recv() {
            let packet = raw.uncompress(self.threshold).unwrap();
            let phase = &mut self.client_phase[client];
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
            C2s::KeepAlive(4),
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

pub(crate) fn new_active_catches_up_after_failover_mid_reconfiguration<W: Wire>() {
    let mut h = Harness::<W>::in_game();
    h.serve(S2c::Ping(1));
    h.serve(S2c::StartConfiguration);
    h.obey(P);
    h.serve(S2c::Raw(1001, 0));
    assert_eq!(h.received(P), [S2c::Raw(1001, 0)]);
    assert_eq!(h.sent(), [C2s::Pong(1), C2s::ConfigurationAck]);

    h.leave(P);
    assert!(h.sent().is_empty());
    h.send_all(S, [C2s::Rot(1.0, 1.0), C2s::Pong(1), C2s::ConfigurationAck]);
    assert!(h.sent().is_empty());
    assert_eq!(
        h.received(S),
        [S2c::Ping(1), S2c::StartConfiguration, S2c::Raw(1001, 0)]
    );

    h.send(S, C2s::Raw(1000));
    assert_eq!(h.sent(), [C2s::Raw(1000)]);
    h.serve(S2c::FinishConfiguration);
    assert_eq!(h.obey(S), [S2c::FinishConfiguration]);
    h.send(S, C2s::Rot(2.0, 2.0));
    assert_eq!(h.sent(), [C2s::FinishConfiguration, C2s::Rot(2.0, 2.0)]);
}

pub(crate) fn failover_after_finish_configuration_drops_lagging_packets<W: Wire>() {
    let mut h = Harness::<W>::in_game();
    h.serve(S2c::StartConfiguration);
    h.obey(P);
    h.obey(S);
    h.serve(S2c::FinishConfiguration);
    h.serve(S2c::Ping(3));
    assert_eq!(h.obey(P), [S2c::FinishConfiguration, S2c::Ping(3)]);
    assert_eq!(
        h.sent(),
        [
            C2s::ConfigurationAck,
            C2s::FinishConfiguration,
            C2s::Pong(3)
        ]
    );

    h.leave(P);
    h.send(S, C2s::Raw(1000));
    assert_eq!(h.obey(S), [S2c::FinishConfiguration, S2c::Ping(3)]);
    assert!(h.sent().is_empty());
    h.send(S, C2s::Rot(1.0, 1.0));
    assert_eq!(h.sent(), [C2s::Rot(1.0, 1.0)]);
}

pub(crate) fn failover_relays_phase_ack_new_active_sent_while_inactive<W: Wire>() {
    let mut h = Harness::<W>::in_game();
    h.serve(S2c::Ping(1));
    h.serve(S2c::StartConfiguration);
    assert_eq!(h.obey(S), [S2c::Ping(1), S2c::StartConfiguration]);
    assert!(h.sent().is_empty());

    h.leave(P);
    assert_eq!(h.sent(), [C2s::Pong(1), C2s::ConfigurationAck]);
    h.serve(S2c::Raw(1001, 0));
    h.serve(S2c::FinishConfiguration);
    assert_eq!(h.obey(S), [S2c::Raw(1001, 0), S2c::FinishConfiguration]);
    assert_eq!(h.sent(), [C2s::FinishConfiguration]);
}

pub(crate) fn failover_relays_what_new_active_sent_after_its_ack<W: Wire>() {
    let mut h = Harness::<W>::in_game();
    h.serve(S2c::StartConfiguration);
    h.obey(P);
    h.obey(S);
    h.serve(S2c::FinishConfiguration);
    assert_eq!(h.obey(S), [S2c::FinishConfiguration]);
    h.send(S, C2s::Rot(1.0, 1.0));
    assert_eq!(h.sent(), [C2s::ConfigurationAck]);

    h.leave(P);
    assert_eq!(h.sent(), [C2s::FinishConfiguration, C2s::Rot(1.0, 1.0)]);
    h.send(S, C2s::Rot(2.0, 2.0));
    assert_eq!(h.sent(), [C2s::Rot(2.0, 2.0)]);
}

pub(crate) fn inactive_phase_ack_is_not_replayed_once_server_got_one<W: Wire>() {
    let mut h = Harness::<W>::in_game();
    h.serve(S2c::StartConfiguration);
    h.obey(S);
    h.send(S, C2s::Raw(1000));
    h.obey(P);
    assert_eq!(h.sent(), [C2s::ConfigurationAck]);

    h.leave(P);
    assert!(h.sent().is_empty());
    h.send(S, C2s::Raw(1001));
    assert_eq!(h.sent(), [C2s::Raw(1001)]);
}

pub(crate) fn failover_before_login_ack_relays_new_active_ack<W: Wire>() {
    let mut h = Harness::<W>::new(Some(0));
    h.send_all(S, [C2s::LoginAck, C2s::Raw(1000)]);
    assert!(h.sent().is_empty());

    h.leave(P);
    assert_eq!(h.sent(), [C2s::LoginAck, C2s::Raw(1000)]);
    h.serve(S2c::FinishConfiguration);
    assert_eq!(h.obey(S), [S2c::FinishConfiguration]);
    assert_eq!(h.sent(), [C2s::FinishConfiguration]);
}

pub(crate) fn failover_after_login_ack_drops_lagging_login<W: Wire>() {
    let mut h = Harness::<W>::new(Some(0));
    h.send_all(P, [C2s::LoginAck, C2s::Raw(1000)]);
    assert_eq!(h.sent(), [C2s::LoginAck, C2s::Raw(1000)]);

    h.leave(P);
    h.send_all(S, [C2s::LoginAck, C2s::Raw(1000)]);
    assert_eq!(h.sent(), [C2s::Raw(1000)]);
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

pub(crate) fn active_keep_alive_reaches_server_once<W: Wire>() {
    let mut h = Harness::<W>::in_game();
    h.serve(S2c::KeepAlive(1));
    assert_eq!(h.received(P), [S2c::KeepAlive(1)]);
    assert_eq!(h.received(S), [S2c::KeepAlive(1)]);
    h.send(S, C2s::KeepAlive(1));
    assert!(h.sent().is_empty());
    h.send(P, C2s::KeepAlive(1));
    assert_eq!(h.sent(), [C2s::KeepAlive(1)]);
}

pub(crate) fn forwarded_keep_alive_is_dropped_after_failover<W: Wire>() {
    let mut h = Harness::<W>::in_game();
    h.serve(S2c::KeepAlive(1));
    h.obey(P);
    assert_eq!(h.sent(), [C2s::KeepAlive(1)]);

    h.leave(P);
    assert!(h.sent().is_empty());
    assert_eq!(h.obey(S), [S2c::KeepAlive(1)]);
    assert!(h.sent().is_empty());

    h.serve(S2c::KeepAlive(2));
    h.obey(S);
    assert_eq!(h.sent(), [C2s::KeepAlive(2)]);
}

pub(crate) fn failover_sends_keep_alive_only_new_active_answered<W: Wire>() {
    let mut h = Harness::<W>::in_game();
    h.serve(S2c::KeepAlive(1));
    h.received(P);
    h.obey(S);
    assert!(h.sent().is_empty());

    h.leave(P);
    assert_eq!(h.sent(), [C2s::KeepAlive(1)]);
}

pub(crate) fn unanswered_keep_alive_is_relayed_after_failover<W: Wire>() {
    let mut h = Harness::<W>::in_game();
    h.serve(S2c::KeepAlive(1));
    h.received(P);
    h.received(S);

    h.leave(P);
    assert!(h.sent().is_empty());
    h.send(S, C2s::KeepAlive(1));
    assert_eq!(h.sent(), [C2s::KeepAlive(1)]);
}

pub(crate) fn failover_keep_alive_follows_server_order<W: Wire>() {
    let mut h = Harness::<W>::in_game();
    let at = spot(1.0, 2.0, 3.0, 0.0, 0.0);
    h.serve(S2c::Ping(1));
    h.serve(S2c::KeepAlive(2));
    h.serve(S2c::Position(5, at));
    h.received(P);
    h.obey(S);
    assert!(h.sent().is_empty());

    h.leave(P);
    assert_eq!(h.sent(), [C2s::Pong(1), C2s::KeepAlive(2), W::ack(5, at)]);
}

pub(crate) fn configuration_keep_alive_is_synced<W: Wire>() {
    let mut h = Harness::<W>::in_game();
    h.serve(S2c::StartConfiguration);
    h.obey(P);
    h.obey(S);
    assert_eq!(h.sent(), [C2s::ConfigurationAck]);

    h.serve(S2c::KeepAlive(1));
    h.obey(S);
    h.obey(P);
    assert_eq!(h.sent(), [C2s::KeepAlive(1)]);

    h.serve(S2c::KeepAlive(2));
    h.obey(S);
    h.received(P);
    assert!(h.sent().is_empty());
    h.leave(P);
    assert_eq!(h.sent(), [C2s::KeepAlive(2)]);
}

pub(crate) fn keep_alive_of_closed_listener_is_not_replayed<W: Wire>() {
    let mut h = Harness::<W>::in_game();
    h.serve(S2c::KeepAlive(1));
    h.serve(S2c::StartConfiguration);
    assert_eq!(h.received(P), [S2c::KeepAlive(1), S2c::StartConfiguration]);
    // A frozen vanilla client defers its answer, see `sendWhen`.
    h.send(P, C2s::ConfigurationAck);
    assert_eq!(h.sent(), [C2s::ConfigurationAck]);
    assert_eq!(h.obey(S), [S2c::KeepAlive(1), S2c::StartConfiguration]);

    h.leave(P);
    assert!(h.sent().is_empty());
}

pub(crate) fn failover_replays_keep_alive_before_phase_ack<W: Wire>() {
    let mut h = Harness::<W>::in_game();
    h.serve(S2c::KeepAlive(1));
    h.serve(S2c::StartConfiguration);
    h.received(P);
    assert_eq!(h.obey(S), [S2c::KeepAlive(1), S2c::StartConfiguration]);
    assert!(h.sent().is_empty());

    h.leave(P);
    assert_eq!(h.sent(), [C2s::KeepAlive(1), C2s::ConfigurationAck]);
}

pub(crate) fn configuration_ping_is_synced<W: Wire>() {
    let mut h = Harness::<W>::in_game();
    h.serve(S2c::StartConfiguration);
    h.obey(P);
    h.obey(S);
    assert_eq!(h.sent(), [C2s::ConfigurationAck]);

    h.serve(S2c::Ping(1));
    h.obey(S);
    h.obey(P);
    assert_eq!(h.sent(), [C2s::Pong(1)]);

    h.serve(S2c::Ping(2));
    h.obey(S);
    h.received(P);
    assert!(h.sent().is_empty());
    h.leave(P);
    assert_eq!(h.sent(), [C2s::Pong(2)]);
}

pub(crate) fn ping_of_closed_listener_is_not_replayed<W: Wire>() {
    let mut h = Harness::<W>::in_game();
    h.serve(S2c::Ping(1));
    h.serve(S2c::StartConfiguration);
    assert_eq!(h.received(P), [S2c::Ping(1), S2c::StartConfiguration]);
    h.send(P, C2s::ConfigurationAck);
    assert_eq!(h.sent(), [C2s::ConfigurationAck]);
    assert_eq!(h.obey(S), [S2c::Ping(1), S2c::StartConfiguration]);

    h.leave(P);
    assert!(h.sent().is_empty());
}

pub(crate) fn failover_replays_pong_before_phase_ack<W: Wire>() {
    let mut h = Harness::<W>::in_game();
    h.serve(S2c::Ping(1));
    h.serve(S2c::StartConfiguration);
    h.received(P);
    assert_eq!(h.obey(S), [S2c::Ping(1), S2c::StartConfiguration]);
    assert!(h.sent().is_empty());

    h.leave(P);
    assert_eq!(h.sent(), [C2s::Pong(1), C2s::ConfigurationAck]);
}

pub(crate) fn failover_relays_pong_after_phase_ack_once<W: Wire>() {
    let mut h = Harness::<W>::in_game();
    h.serve(S2c::StartConfiguration);
    h.serve(S2c::Ping(1));
    h.received(P);
    assert_eq!(h.obey(S), [S2c::StartConfiguration, S2c::Ping(1)]);
    assert!(h.sent().is_empty());

    h.leave(P);
    assert_eq!(h.sent(), [C2s::ConfigurationAck, C2s::Pong(1)]);
}

pub(crate) fn pong_to_listener_server_reached_later_is_replayed<W: Wire>() {
    let mut h = Harness::<W>::in_game();
    h.serve(S2c::StartConfiguration);
    h.serve(S2c::Ping(1));
    assert_eq!(h.obey(S), [S2c::StartConfiguration, S2c::Ping(1)]);
    assert_eq!(h.received(P), [S2c::StartConfiguration]);
    h.send(P, C2s::ConfigurationAck);
    assert_eq!(h.sent(), [C2s::ConfigurationAck]);

    // The primary leaves before it handles the ping released by its ack.
    h.leave(P);
    assert_eq!(h.sent(), [C2s::Pong(1)]);
}

pub(crate) fn lagging_pong_with_reused_id_is_not_counted<W: Wire>() {
    let mut h = Harness::<W>::in_game();
    h.serve(S2c::Ping(1));
    h.serve(S2c::StartConfiguration);
    h.obey(P);
    h.serve(S2c::Ping(1));
    assert_eq!(h.received(P), [S2c::Ping(1)]);
    assert_eq!(h.sent(), [C2s::Pong(1), C2s::ConfigurationAck]);

    // The secondary answers the game listener's ping after the new listener sent the same id.
    h.send_all(S, [C2s::Pong(1), C2s::ConfigurationAck]);
    h.received(S);
    assert!(h.sent().is_empty());
    h.leave(P);
    assert!(h.sent().is_empty());
    h.send(S, C2s::Pong(1));
    assert_eq!(h.sent(), [C2s::Pong(1)]);
}

pub(crate) fn lagging_pongs_are_dropped_after_failover<W: Wire>() {
    let mut h = Harness::<W>::in_game();
    h.serve(S2c::Ping(1));
    h.serve(S2c::StartConfiguration);
    h.serve(S2c::Ping(2));
    assert_eq!(
        h.obey(P),
        [S2c::Ping(1), S2c::StartConfiguration, S2c::Ping(2)]
    );
    assert_eq!(
        h.sent(),
        [C2s::Pong(1), C2s::ConfigurationAck, C2s::Pong(2)]
    );

    h.leave(P);
    assert_eq!(
        h.obey(S),
        [S2c::Ping(1), S2c::StartConfiguration, S2c::Ping(2)]
    );
    assert!(h.sent().is_empty());
    h.serve(S2c::Ping(3));
    h.obey(S);
    assert_eq!(h.sent(), [C2s::Pong(3)]);
}

const BATCH: S2c = S2c::ChunkBatchFinished(4);

pub(crate) fn active_chunk_batch_answer_reaches_server_once<W: Wire>() {
    let mut h = Harness::<W>::in_game();
    h.serve(BATCH);
    assert_eq!(h.received(P), [BATCH]);
    assert_eq!(h.received(S), [BATCH]);
    h.send(S, C2s::ChunkBatchReceived(20.0));
    assert!(h.sent().is_empty());
    h.send(P, C2s::ChunkBatchReceived(5.0));
    assert_eq!(h.sent(), [C2s::ChunkBatchReceived(5.0)]);
}

pub(crate) fn forwarded_chunk_batch_answer_is_dropped_after_failover<W: Wire>() {
    let mut h = Harness::<W>::in_game();
    h.serve(BATCH);
    h.obey(P);
    assert_eq!(h.sent(), [C2s::ChunkBatchReceived(CHUNKS_PER_TICK)]);

    h.leave(P);
    assert!(h.sent().is_empty());
    assert_eq!(h.obey(S), [BATCH]);
    assert!(h.sent().is_empty());

    h.serve(BATCH);
    h.obey(S);
    assert_eq!(h.sent(), [C2s::ChunkBatchReceived(CHUNKS_PER_TICK)]);
}

pub(crate) fn failover_replays_chunk_batch_answer_of_new_active<W: Wire>() {
    let mut h = Harness::<W>::in_game();
    h.serve(BATCH);
    h.serve(BATCH);
    h.received(P);
    h.received(S);
    h.send(P, C2s::ChunkBatchReceived(5.0));
    h.send_all(
        S,
        [C2s::ChunkBatchReceived(20.0), C2s::ChunkBatchReceived(21.0)],
    );
    assert_eq!(h.sent(), [C2s::ChunkBatchReceived(5.0)]);

    h.leave(P);
    assert_eq!(h.sent(), [C2s::ChunkBatchReceived(21.0)]);
}

pub(crate) fn unanswered_chunk_batch_is_relayed_after_failover<W: Wire>() {
    let mut h = Harness::<W>::in_game();
    h.serve(BATCH);
    h.received(P);
    h.received(S);

    h.leave(P);
    assert!(h.sent().is_empty());
    h.send(S, C2s::ChunkBatchReceived(20.0));
    assert_eq!(h.sent(), [C2s::ChunkBatchReceived(20.0)]);
}

pub(crate) fn chunk_batch_answer_reaches_server_after_inactive_left<W: Wire>() {
    let mut h = Harness::<W>::in_game();
    h.serve(BATCH);
    h.obey(S);
    h.leave(S);
    h.send(P, C2s::ChunkBatchReceived(5.0));
    assert_eq!(h.sent(), [C2s::ChunkBatchReceived(5.0)]);
}

pub(crate) fn failover_chunk_batch_answer_follows_server_order<W: Wire>() {
    let mut h = Harness::<W>::in_game();
    let at = spot(1.0, 2.0, 3.0, 0.0, 0.0);
    h.serve(S2c::Ping(1));
    h.serve(BATCH);
    h.serve(S2c::KeepAlive(2));
    h.serve(S2c::Position(5, at));
    h.received(P);
    h.obey(S);
    assert!(h.sent().is_empty());

    h.leave(P);
    assert_eq!(
        h.sent(),
        [
            C2s::Pong(1),
            C2s::ChunkBatchReceived(CHUNKS_PER_TICK),
            C2s::KeepAlive(2),
            W::ack(5, at)
        ]
    );
}

pub(crate) fn chunk_batches_of_closed_game_listener_are_forgotten<W: Wire>() {
    let mut h = Harness::<W>::in_game();
    h.serve(BATCH);
    h.serve(S2c::StartConfiguration);
    h.obey(P);
    h.serve(S2c::FinishConfiguration);
    h.obey(P);
    h.serve(BATCH);
    assert_eq!(h.received(P), [BATCH]);
    assert_eq!(
        h.sent(),
        [
            C2s::ChunkBatchReceived(CHUNKS_PER_TICK),
            C2s::ConfigurationAck,
            C2s::FinishConfiguration
        ]
    );

    // The secondary still answers the first listener's batch.
    assert_eq!(h.received(S), [BATCH, S2c::StartConfiguration]);
    h.send(S, C2s::ChunkBatchReceived(20.0));
    h.leave(P);
    assert!(h.sent().is_empty());
    h.send(S, C2s::ConfigurationAck);
    assert_eq!(h.obey(S), [S2c::FinishConfiguration, BATCH]);
    assert_eq!(h.sent(), [C2s::ChunkBatchReceived(CHUNKS_PER_TICK)]);
}

const PACK: S2c = S2c::ResourcePackPush(0);

pub(crate) fn resource_pack_answers_reach_server_once<W: Wire>() {
    let mut h = Harness::<W>::in_configuration();
    h.serve(PACK);
    assert_eq!(h.obey(S), [PACK]);
    assert!(h.sent().is_empty());
    assert_eq!(h.obey(P), [PACK]);
    assert_eq!(h.sent(), W::respond(PACK));

    h.serve(S2c::FinishConfiguration);
    h.obey(S);
    h.obey(P);
    assert_eq!(h.sent(), [C2s::FinishConfiguration]);
}

pub(crate) fn pack_answers_are_not_repeated_after_failover<W: Wire>() {
    let mut h = Harness::<W>::in_configuration();
    h.serve(PACK);
    h.obey(P);
    assert_eq!(h.sent(), W::respond(PACK));

    h.leave(P);
    assert!(h.sent().is_empty());
    assert_eq!(h.obey(S), [PACK]);
    assert!(h.sent().is_empty());
    h.serve(S2c::FinishConfiguration);
    h.obey(S);
    assert_eq!(h.sent(), [C2s::FinishConfiguration]);
}

pub(crate) fn failover_replays_pack_answers_new_active_sent<W: Wire>() {
    let mut h = Harness::<W>::in_configuration();
    h.serve(PACK);
    h.received(P);
    h.obey(S);
    assert!(h.sent().is_empty());

    h.leave(P);
    assert_eq!(h.sent(), W::respond(PACK));
}

pub(crate) fn failover_mid_pack_continues_from_server_answers<W: Wire>() {
    let mut h = Harness::<W>::in_configuration();
    let answers = W::respond(PACK);
    let (last, rest) = answers.split_last().unwrap();
    h.serve(PACK);
    h.received(P);
    h.received(S);
    h.send(P, answers[0]);
    h.send_all(S, rest.iter().copied());
    assert_eq!(h.sent(), [answers[0]]);

    h.leave(P);
    assert_eq!(h.sent(), rest[1..]);
    h.send(S, *last);
    assert_eq!(h.sent(), [*last]);
}

pub(crate) fn failover_relays_decline_after_partial_answer<W: Wire>() {
    let mut h = Harness::<W>::in_configuration();
    h.serve(PACK);
    h.received(P);
    h.received(S);
    h.send(P, C2s::ResourcePack(0, Pack::Accepted));
    h.send(S, C2s::ResourcePack(0, Pack::Declined));
    assert_eq!(h.sent(), [C2s::ResourcePack(0, Pack::Accepted)]);

    h.leave(P);
    assert_eq!(h.sent(), [C2s::ResourcePack(0, Pack::Declined)]);
}

pub(crate) fn decline_after_finished_pack_is_dropped<W: Wire>() {
    let mut h = Harness::<W>::in_configuration();
    h.serve(PACK);
    h.obey(P);
    h.sent();
    h.received(S);
    h.send(S, C2s::ResourcePack(0, Pack::Declined));

    h.leave(P);
    assert!(h.sent().is_empty());
    h.serve(S2c::FinishConfiguration);
    h.obey(S);
    assert_eq!(h.sent(), [C2s::FinishConfiguration]);
}

pub(crate) fn pack_answers_after_inactive_left_reach_server<W: Wire>() {
    let mut h = Harness::<W>::in_configuration();
    h.serve(PACK);
    h.received(S);
    h.leave(S);
    assert_eq!(h.obey(P), [PACK]);
    assert_eq!(h.sent(), W::respond(PACK));
}

pub(crate) fn tasks_of_finished_listener_are_forgotten<W: Wire>() {
    let mut h = Harness::<W>::in_game();
    h.serve(S2c::StartConfiguration);
    h.obey(P);
    h.obey(S);
    h.serve(PACK);
    h.obey(P);
    // The secondary's pack prompt is still open when configuration finishes.
    assert_eq!(h.received(S), [PACK]);
    h.serve(S2c::FinishConfiguration);
    h.obey(P);
    assert_eq!(h.received(S), [S2c::FinishConfiguration]);
    h.send(S, C2s::FinishConfiguration);
    assert_eq!(
        h.sent(),
        [
            vec![C2s::ConfigurationAck],
            W::respond(PACK),
            vec![C2s::FinishConfiguration]
        ]
        .concat()
    );

    h.serve(S2c::StartConfiguration);
    h.obey(P);
    h.obey(S);
    assert_eq!(h.sent(), [C2s::ConfigurationAck]);
    h.serve(PACK);
    h.received(P);
    assert_eq!(h.obey(S), [PACK]);
    h.leave(P);
    assert_eq!(h.sent(), W::respond(PACK));
}

pub(crate) fn lagging_answer_to_earlier_listener_is_not_counted<W: Wire>() {
    let mut h = Harness::<W>::in_configuration();
    h.serve(PACK);
    h.obey(P);
    h.serve(S2c::FinishConfiguration);
    h.obey(P);
    h.serve(S2c::StartConfiguration);
    h.obey(P);
    h.serve(PACK);
    h.obey(P);
    assert_eq!(
        h.sent(),
        [
            W::respond(PACK),
            vec![C2s::FinishConfiguration, C2s::ConfigurationAck],
            W::respond(PACK)
        ]
        .concat()
    );

    // The secondary still answers the first listener's pack.
    h.leave(P);
    h.obey(S);
    assert!(h.sent().is_empty());
}

pub(crate) fn known_packs_answer_reaches_server_once<W: Wire>() {
    let mut h = Harness::<W>::in_configuration();
    h.serve(S2c::SelectKnownPacks);
    h.obey(P);
    assert_eq!(h.sent(), [C2s::KnownPacks]);

    h.leave(P);
    assert_eq!(h.obey(S), [S2c::SelectKnownPacks]);
    assert!(h.sent().is_empty());
}

pub(crate) fn failover_replays_known_packs_answer<W: Wire>() {
    let mut h = Harness::<W>::in_configuration();
    h.serve(S2c::SelectKnownPacks);
    h.received(P);
    h.obey(S);
    assert!(h.sent().is_empty());

    h.leave(P);
    assert_eq!(h.sent(), [C2s::KnownPacks]);
    h.serve(S2c::FinishConfiguration);
    h.obey(S);
    assert_eq!(h.sent(), [C2s::FinishConfiguration]);
}

pub(crate) fn failover_task_answers_follow_server_order<W: Wire>() {
    let mut h = Harness::<W>::in_configuration();
    h.serve(S2c::SelectKnownPacks);
    h.obey(P);
    h.obey(S);
    assert_eq!(h.sent(), [C2s::KnownPacks]);
    h.serve(S2c::KeepAlive(1));
    h.serve(PACK);
    h.received(P);
    h.obey(S);
    assert!(h.sent().is_empty());

    h.leave(P);
    assert_eq!(
        h.sent(),
        [vec![C2s::KeepAlive(1)], W::respond(PACK)].concat()
    );
}

pub(crate) fn code_of_conduct_is_accepted_once<W: Wire>() {
    let mut h = Harness::<W>::in_configuration();
    h.serve(S2c::SelectKnownPacks);
    h.serve(S2c::CodeOfConduct);
    h.obey(P);
    assert_eq!(h.sent(), [C2s::KnownPacks, C2s::AcceptCodeOfConduct]);

    h.leave(P);
    h.obey(S);
    assert!(h.sent().is_empty());
}

pub(crate) fn failover_replays_code_of_conduct_acceptance<W: Wire>() {
    let mut h = Harness::<W>::in_configuration();
    h.serve(S2c::SelectKnownPacks);
    h.obey(P);
    h.obey(S);
    h.serve(S2c::CodeOfConduct);
    h.received(P);
    h.obey(S);
    assert_eq!(h.sent(), [C2s::KnownPacks]);

    h.leave(P);
    assert_eq!(h.sent(), [C2s::AcceptCodeOfConduct]);
}

pub(crate) fn failover_replays_answers_per_pack<W: Wire>() {
    let mut h = Harness::<W>::in_configuration();
    let (first, second) = (S2c::ResourcePackPush(1), S2c::ResourcePackPush(2));
    h.serve(first);
    h.serve(second);
    h.received(P);
    h.obey(S);
    h.send_all(P, W::respond(first));
    h.send(P, C2s::ResourcePack(2, Pack::Accepted));
    assert_eq!(
        h.sent(),
        [
            W::respond(first),
            vec![C2s::ResourcePack(2, Pack::Accepted)]
        ]
        .concat()
    );

    h.leave(P);
    assert_eq!(
        h.sent(),
        [
            C2s::ResourcePack(2, Pack::Downloaded),
            C2s::ResourcePack(2, Pack::Loaded)
        ]
    );
}

pub(crate) fn pack_pushed_again_is_answered_again<W: Wire>() {
    let mut h = Harness::<W>::in_configuration();
    h.serve(PACK);
    h.obey(P);
    assert_eq!(h.sent(), W::respond(PACK));
    assert_eq!(h.received(S), [PACK]);

    // The secondary only answers the second push of the same pack.
    h.serve(PACK);
    h.received(P);
    assert_eq!(h.received(S), [PACK]);
    h.send_all(S, W::respond(PACK));
    h.leave(P);
    assert_eq!(h.sent(), W::respond(PACK));
}

/// A test signature told apart by its first byte, so windows get distinct checksums.
pub(crate) fn signature(id: u8) -> [u8; 256] {
    let mut bytes = [0; 256];
    bytes[0] = id;
    bytes
}

/// `MessageSignature.checksum`.
fn signature_hash(id: u8) -> i32 {
    signature(id).iter().fold(1i32, |hash, b| {
        hash.wrapping_mul(31).wrapping_add(*b as i8 as i32)
    })
}

/// `LastSeenMessages.computeChecksum`.
fn checksum(signatures: &[u8]) -> u8 {
    let hash = signatures.iter().fold(1i32, |hash, s| {
        hash.wrapping_mul(31).wrapping_add(signature_hash(*s))
    });
    match hash as u8 {
        0 => 1,
        b => b,
    }
}

/// A vanilla client's `LastSeenMessagesTracker`.
#[derive(Default, Clone)]
pub(crate) struct ClientChat {
    tracked: [Option<(u8, bool)>; 20],
    tail: usize,
    offset: i32,
    last: Option<u8>,
}

impl ClientChat {
    /// `ClientPacketListener.markMessageAsProcessed`, returning the ack it may send.
    fn see(&mut self, signature: u8, shown: bool) -> Option<C2s> {
        if self.last == Some(signature) {
            return None;
        }
        self.last = Some(signature);
        self.tracked[self.tail] = shown.then_some((signature, true));
        self.tail = (self.tail + 1) % 20;
        self.offset += 1;
        if self.offset > 64 { self.ack() } else { None }
    }

    /// `ClientPacketListener.sendChatAcknowledgement`.
    fn ack(&mut self) -> Option<C2s> {
        let offset = std::mem::take(&mut self.offset);
        (offset > 0).then_some(C2s::ChatAck(offset))
    }

    /// `LastSeenMessagesTracker.generateAndApplyUpdate`.
    fn update(&mut self, with_checksum: bool) -> Seen {
        let mut acked = 0;
        let mut signatures = vec![];
        for i in 0..20 {
            let slot = &mut self.tracked[(self.tail + i) % 20];
            if let Some((signature, _)) = *slot {
                acked |= 1 << i;
                signatures.push(signature);
                *slot = Some((signature, false));
            }
        }
        Seen {
            offset: std::mem::take(&mut self.offset),
            acked,
            checksum: if with_checksum {
                checksum(&signatures)
            } else {
                0
            },
        }
    }
}

/// The server's `LastSeenMessagesValidator` and chat order; an error is a kick.
pub(crate) struct ServerChat {
    tracked: Vec<Option<(u8, bool)>>,
    last: Option<u8>,
    /// `lastChatTimeStamp`, from `Instant.EPOCH`.
    last_time_stamp: i64,
}

impl Default for ServerChat {
    fn default() -> Self {
        Self {
            tracked: vec![None; 20],
            last: None,
            last_time_stamp: 0,
        }
    }
}

impl ServerChat {
    fn add(&mut self, signature: u8) {
        if self.last != Some(signature) {
            self.tracked.push(Some((signature, true)));
            self.last = Some(signature);
        }
    }

    fn apply_offset(&mut self, offset: i32) -> Result<(), String> {
        let max = self.tracked.len() - 20;
        if offset < 0 || offset as usize > max {
            return Err(format!("advanced by {offset}, expected at most {max}"));
        }
        self.tracked.drain(..offset as usize);
        Ok(())
    }

    fn apply_update(&mut self, seen: Seen) -> Result<(), String> {
        self.apply_offset(seen.offset)?;
        if seen.acked >> 20 != 0 {
            return Err(format!("acked {:#x} is wider than the window", seen.acked));
        }
        let mut signatures = vec![];
        for (i, slot) in self.tracked.iter_mut().take(20).enumerate() {
            match (seen.acked >> i & 1 == 1, *slot) {
                (true, None) => return Err(format!("acked unknown message at {i}")),
                (true, Some((signature, _))) => {
                    *slot = Some((signature, false));
                    signatures.push(signature);
                }
                (false, Some((_, false))) => {
                    return Err(format!("ignored acknowledged message at {i}"));
                }
                (false, _) => *slot = None,
            }
        }
        if seen.checksum != 0 && seen.checksum != checksum(&signatures) {
            return Err("checksum mismatch".into());
        }
        Ok(())
    }

    /// `ServerGamePacketListenerImpl.updateChatOrder`; an error is an `out_of_order_chat` kick.
    fn update_chat_order(&mut self, time_stamp: i64) -> Result<(), String> {
        if time_stamp < self.last_time_stamp {
            return Err(format!(
                "out_of_order_chat: {time_stamp} is before {}",
                self.last_time_stamp
            ));
        }
        self.last_time_stamp = time_stamp;
        Ok(())
    }

    fn handle(&mut self, packet: C2s, chat_order: bool) -> Result<(), String> {
        match packet {
            C2s::ChatAck(offset) => self.apply_offset(offset),
            C2s::Chat(seen, time_stamp) | C2s::SignedCommand(seen, time_stamp) => {
                if chat_order {
                    self.update_chat_order(time_stamp)?;
                }
                self.apply_update(seen)
            }
            _ => Ok(()),
        }
    }
}

/// Signed chat through the proxy, with vanilla chat state on the clients and the server.
pub(crate) struct ChatLab<W> {
    pub h: Harness<W>,
    server: ServerChat,
    clients: PerClient<ClientChat>,
    /// Each client's clock, in epoch millis.
    pub clocks: PerClient<i64>,
}

impl<W: Wire> ChatLab<W> {
    pub fn new() -> Self {
        Self {
            h: Harness::in_game(),
            server: ServerChat::default(),
            clients: PerClient::default(),
            clocks: PerClient::default(),
        }
    }

    /// The server sends signed player messages.
    pub fn broadcast(&mut self, signatures: impl IntoIterator<Item = u8>) {
        for signature in signatures {
            self.h.serve(S2c::PlayerChat(Some(signature)));
            self.server.add(signature);
        }
    }

    /// The client shows what it got, except messages in `hidden`, acking like vanilla.
    pub fn read_hiding(&mut self, client: ClientId, hidden: &[u8]) {
        for packet in self.h.received(client) {
            if let S2c::PlayerChat(Some(signature)) = packet
                && let Some(ack) = self.clients[client].see(signature, !hidden.contains(&signature))
            {
                self.h.send(client, ack);
            }
        }
    }

    pub fn read(&mut self, client: ClientId) {
        self.read_hiding(client, &[]);
    }

    pub fn chat(&mut self, client: ClientId) -> Seen {
        let seen = self.clients[client].update(W::CHECKSUM);
        self.h.send(client, C2s::Chat(seen, self.clocks[client]));
        seen
    }

    pub fn command(&mut self, client: ClientId) -> Seen {
        let seen = self.clients[client].update(W::CHECKSUM);
        self.h
            .send(client, C2s::SignedCommand(seen, self.clocks[client]));
        seen
    }

    /// What the server got, after its validator accepted every packet.
    pub fn check(&mut self) -> Vec<C2s> {
        let sent = self.h.sent();
        for packet in &sent {
            if let Err(e) = self.server.handle(*packet, W::CHAT_ORDER) {
                panic!("server kicked on {packet:?}: {e}");
            }
        }
        sent
    }
}

pub(crate) fn active_chat_acks_pass_unchanged<W: Wire>() {
    let mut lab = ChatLab::<W>::new();
    lab.broadcast(1..=3);
    lab.read(P);
    lab.read(S);
    let seen = lab.chat(P);
    assert_eq!(lab.check(), [C2s::Chat(seen, 0)]);
    lab.broadcast(4..=70);
    lab.read(P);
    assert_eq!(lab.check(), [C2s::ChatAck(65)]);
    let seen = lab.command(P);
    assert_eq!(lab.check(), [C2s::SignedCommand(seen, 0)]);
}

pub(crate) fn inactive_chat_acks_never_reach_server<W: Wire>() {
    let mut lab = ChatLab::<W>::new();
    lab.broadcast(1..=70);
    lab.read(S);
    lab.chat(S);
    lab.command(S);
    assert!(lab.check().is_empty());
}

pub(crate) fn chat_after_failover_matches_server_window<W: Wire>() {
    let mut lab = ChatLab::<W>::new();
    lab.broadcast(1..=3);
    lab.read(P);
    lab.read(S);
    lab.chat(P);
    lab.check();

    lab.h.leave(P);
    let seen = lab.chat(S);
    assert_eq!(lab.check(), [C2s::Chat(Seen { offset: 0, ..seen }, 0)]);
    lab.broadcast(4..=5);
    lab.read(S);
    let seen = lab.command(S);
    assert_eq!(lab.check(), [C2s::SignedCommand(seen, 0)]);
}

pub(crate) fn chat_ack_after_failover_matches_server_window<W: Wire>() {
    let mut lab = ChatLab::<W>::new();
    lab.broadcast(1..=10);
    lab.read(P);
    lab.read(S);
    lab.chat(P);
    lab.check();

    lab.h.leave(P);
    lab.broadcast(11..=70);
    lab.read(S);
    assert_eq!(lab.check(), [C2s::ChatAck(55)]);
    let seen = lab.chat(S);
    assert_eq!(lab.check(), [C2s::Chat(seen, 0)]);
}

pub(crate) fn chat_ack_server_already_got_is_dropped<W: Wire>() {
    let mut lab = ChatLab::<W>::new();
    lab.broadcast(1..=65);
    lab.read(P);
    assert_eq!(lab.check(), [C2s::ChatAck(65)]);
    lab.broadcast(66..=67);
    lab.read(P);
    lab.chat(P);
    lab.check();

    lab.h.leave(P);
    lab.read(S);
    assert!(lab.check().is_empty());
    let seen = lab.chat(S);
    assert_eq!(lab.check(), [C2s::Chat(Seen { offset: 0, ..seen }, 0)]);
}

pub(crate) fn lagging_client_keeps_acks_server_got<W: Wire>() {
    let mut lab = ChatLab::<W>::new();
    lab.broadcast(1..=3);
    lab.read(P);
    lab.chat(P);
    lab.check();

    // The secondary chats before it shows the messages the primary acked.
    lab.h.leave(P);
    assert_eq!(lab.chat(S).acked, 0);
    lab.check();
    lab.read(S);
    lab.broadcast([4]);
    lab.read(S);
    let seen = lab.chat(S);
    assert_eq!(lab.check(), [C2s::Chat(Seen { offset: 1, ..seen }, 0)]);
}

/// The checksum a version puts on the wire for these acked messages.
fn wire_checksum<W: Wire>(signatures: &[u8]) -> u8 {
    if W::CHECKSUM { checksum(signatures) } else { 0 }
}

pub(crate) fn lagging_client_checksum_matches_server_window<W: Wire>() {
    let mut lab = ChatLab::<W>::new();
    lab.broadcast(1..=65);
    lab.read(P);
    lab.read(S);
    assert_eq!(lab.check(), [C2s::ChatAck(65)]);
    lab.broadcast([66]);
    lab.read(P);
    lab.chat(P);
    lab.check();

    // The secondary acks the same 20 slots as the server, one message earlier.
    lab.h.leave(P);
    let seen = lab.chat(S);
    assert_eq!((seen.offset, seen.acked), (0, (1 << 20) - 1));
    let synced = Seen {
        checksum: wire_checksum::<W>(&(47..=66).collect::<Vec<_>>()),
        ..seen
    };
    assert_eq!(lab.check(), [C2s::Chat(synced, 0)]);
}

pub(crate) fn hidden_message_acked_by_old_active_stays_acked<W: Wire>() {
    let mut lab = ChatLab::<W>::new();
    lab.broadcast(1..=3);
    lab.read(P);
    lab.read_hiding(S, &[2]);
    lab.chat(S);
    lab.chat(P);
    lab.check();

    // The window lines up, but only the primary acked the second message.
    lab.h.leave(P);
    let seen = lab.chat(S);
    assert_eq!((seen.offset, seen.acked), (0, 0b101 << 17));
    let synced = Seen {
        offset: 0,
        acked: 0b111 << 17,
        checksum: wire_checksum::<W>(&[1, 2, 3]),
    };
    assert_eq!(lab.check(), [C2s::Chat(synced, 0)]);
}

pub(crate) fn message_ignored_by_old_active_is_not_acked<W: Wire>() {
    let mut lab = ChatLab::<W>::new();
    lab.broadcast(1..=3);
    lab.read_hiding(P, &[2]);
    lab.read(S);
    lab.chat(S);
    lab.chat(P);
    lab.check();

    lab.h.leave(P);
    let seen = lab.chat(S);
    assert_eq!((seen.offset, seen.acked), (0, 0b111 << 17));
    let synced = Seen {
        offset: 0,
        acked: 0b101 << 17,
        checksum: wire_checksum::<W>(&[1, 3]),
    };
    assert_eq!(lab.check(), [C2s::Chat(synced, 0)]);
}

pub(crate) fn unsigned_chat_is_not_tracked<W: Wire>() {
    let mut lab = ChatLab::<W>::new();
    for _ in 0..70 {
        lab.h.serve(S2c::PlayerChat(None));
    }
    lab.read(P);
    lab.read(S);
    lab.h.leave(P);
    let seen = lab.chat(S);
    assert_eq!(seen.offset, 0);
    assert_eq!(lab.check(), [C2s::Chat(seen, 0)]);
}

/// The time stamp the server gets: raised to `last` on versions that check chat order.
fn ordered<W: Wire>(time_stamp: i64, last: i64) -> i64 {
    if W::CHAT_ORDER {
        time_stamp.max(last)
    } else {
        time_stamp
    }
}

pub(crate) fn chat_after_failover_to_lagging_clock_is_in_order<W: Wire>() {
    let mut lab = ChatLab::<W>::new();
    lab.clocks = PerClient::new(10_000, 4_000);
    lab.broadcast(1..=3);
    lab.read(P);
    lab.read(S);
    let seen = lab.chat(P);
    assert_eq!(lab.check(), [C2s::Chat(seen, 10_000)]);

    // The secondary's clock is 6 seconds behind, and only 1 more passes.
    lab.h.leave(P);
    let seen = lab.chat(S);
    let time_stamp = ordered::<W>(4_000, 10_000);
    assert_eq!(
        lab.check(),
        [C2s::Chat(Seen { offset: 0, ..seen }, time_stamp)]
    );
    lab.clocks[S] = 5_000;
    let seen = lab.command(S);
    let time_stamp = ordered::<W>(5_000, 10_000);
    assert_eq!(lab.check(), [C2s::SignedCommand(seen, time_stamp)]);
    lab.clocks[S] = 12_000;
    let seen = lab.chat(S);
    assert_eq!(lab.check(), [C2s::Chat(seen, 12_000)]);
}

pub(crate) fn chat_time_stamps_in_order_pass_unchanged<W: Wire>() {
    let mut lab = ChatLab::<W>::new();
    lab.clocks = PerClient::new(1_000, 1_500);
    let seen = lab.chat(P);
    assert_eq!(lab.check(), [C2s::Chat(seen, 1_000)]);
    // The inactive client's chat never reaches the server, so it moves nothing.
    lab.chat(S);
    assert!(lab.check().is_empty());
    lab.clocks[P] = 1_200;
    let seen = lab.command(P);
    assert_eq!(lab.check(), [C2s::SignedCommand(seen, 1_200)]);

    lab.h.leave(P);
    let seen = lab.chat(S);
    assert_eq!(lab.check(), [C2s::Chat(seen, 1_500)]);
    // `updateChatOrder` only rejects `isBefore`, so an equal one passes.
    let seen = lab.command(S);
    assert_eq!(lab.check(), [C2s::SignedCommand(seen, 1_500)]);
}

pub(crate) fn chat_window_restarts_with_game_listener<W: Wire>() {
    let mut lab = ChatLab::<W>::new();
    lab.broadcast(1..=3);
    lab.read(P);
    lab.read(S);
    lab.chat(P);
    lab.check();
    lab.broadcast(4..=5);

    // `handleConfigurationStart` acks what the client saw before it acks the switch.
    lab.h.serve(S2c::StartConfiguration);
    for client in [P, S] {
        lab.read(client);
        let ack = lab.clients[client].ack().unwrap();
        lab.h.send_all(client, [ack, C2s::ConfigurationAck]);
    }
    assert_eq!(lab.check(), [C2s::ChatAck(2), C2s::ConfigurationAck]);
    lab.h.serve(S2c::FinishConfiguration);
    lab.h.obey(P);
    lab.h.obey(S);
    assert_eq!(lab.check(), [C2s::FinishConfiguration]);
    lab.server = ServerChat::default();
    lab.clients = PerClient::default();

    lab.broadcast(6..=8);
    lab.read(P);
    lab.read(S);
    let seen = lab.chat(P);
    assert_eq!(lab.check(), [C2s::Chat(seen, 0)]);
    lab.h.leave(P);
    let seen = lab.chat(S);
    assert_eq!(lab.check(), [C2s::Chat(Seen { offset: 0, ..seen }, 0)]);
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
    (configuration $($extra:ident)*) => {
        $crate::controller::tests::controller_tests!(signed_chat
            nothing_is_mirrored_during_reconfiguration
            start_configuration_holds_packets_until_ack
            finish_configuration_holds_packets_until_ack
            held_packets_stop_at_next_phase_switch
            failover_keeps_packets_held_for_new_active
            failover_during_reconfiguration_keeps_server_phase
            new_active_catches_up_after_failover_mid_reconfiguration
            failover_after_finish_configuration_drops_lagging_packets
            failover_relays_phase_ack_new_active_sent_while_inactive
            failover_relays_what_new_active_sent_after_its_ack
            inactive_phase_ack_is_not_replayed_once_server_got_one
            failover_before_login_ack_relays_new_active_ack
            failover_after_login_ack_drops_lagging_login
            own_ack_after_reconfiguration_is_dropped
            configuration_keep_alive_is_synced
            keep_alive_of_closed_listener_is_not_replayed
            failover_replays_keep_alive_before_phase_ack
            configuration_ping_is_synced
            ping_of_closed_listener_is_not_replayed
            failover_replays_pong_before_phase_ack
            failover_relays_pong_after_phase_ack_once
            pong_to_listener_server_reached_later_is_replayed
            lagging_pongs_are_dropped_after_failover
            lagging_pong_with_reused_id_is_not_counted
            resource_pack_answers_reach_server_once
            pack_answers_are_not_repeated_after_failover
            failover_replays_pack_answers_new_active_sent
            failover_mid_pack_continues_from_server_answers
            failover_relays_decline_after_partial_answer
            decline_after_finished_pack_is_dropped
            pack_answers_after_inactive_left_reach_server
            tasks_of_finished_listener_are_forgotten
            lagging_answer_to_earlier_listener_is_not_counted
            active_chunk_batch_answer_reaches_server_once
            forwarded_chunk_batch_answer_is_dropped_after_failover
            failover_replays_chunk_batch_answer_of_new_active
            unanswered_chunk_batch_is_relayed_after_failover
            chunk_batch_answer_reaches_server_after_inactive_left
            failover_chunk_batch_answer_follows_server_order
            chunk_batches_of_closed_game_listener_are_forgotten
            chat_window_restarts_with_game_listener
            $($extra)*
        );
    };
    (signed_chat $($extra:ident)*) => {
        $crate::controller::tests::controller_tests!(@common
            active_chat_acks_pass_unchanged
            inactive_chat_acks_never_reach_server
            chat_after_failover_matches_server_window
            chat_ack_after_failover_matches_server_window
            chat_ack_server_already_got_is_dropped
            lagging_client_keeps_acks_server_got
            lagging_client_checksum_matches_server_window
            hidden_message_acked_by_old_active_stays_acked
            message_ignored_by_old_active_is_not_acked
            unsigned_chat_is_not_tracked
            chat_after_failover_to_lagging_clock_is_in_order
            chat_time_stamps_in_order_pass_unchanged
            $($extra)*
        );
    };
    (pack_ids $($rest:ident)*) => {
        $crate::controller::tests::controller_tests!(configuration
            failover_replays_answers_per_pack
            pack_pushed_again_is_answered_again
            $($rest)*
        );
    };
    (known_packs $($rest:ident)*) => {
        $crate::controller::tests::controller_tests!(pack_ids
            known_packs_answer_reaches_server_once
            failover_replays_known_packs_answer
            failover_task_answers_follow_server_order
            $($rest)*
        );
    };
    (code_of_conduct) => {
        $crate::controller::tests::controller_tests!(known_packs
            code_of_conduct_is_accepted_once
            failover_replays_code_of_conduct_acceptance
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
            active_keep_alive_reaches_server_once
            forwarded_keep_alive_is_dropped_after_failover
            failover_sends_keep_alive_only_new_active_answered
            unanswered_keep_alive_is_relayed_after_failover
            failover_keep_alive_follows_server_order
            $($extra)*
        );
    };
}
pub(crate) use controller_tests;
