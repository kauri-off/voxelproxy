pub mod v1_16_5;
pub mod v1_17;
pub mod v1_17_1;
pub mod v1_18_1;
pub mod v1_18_2;
pub mod v1_19;
pub mod v1_19_2;
pub mod v1_19_3;
pub mod v1_19_4;
pub mod v1_20_1;
pub mod v1_20_2;
pub mod v1_20_4;
pub mod v1_20_6;
pub mod v1_21_1;
pub mod v1_21_10;
pub mod v1_21_11;
pub mod v1_21_3;
pub mod v1_21_4;
pub mod v1_21_5;
pub mod v1_21_6;
pub mod v1_21_8;
pub mod v26_1_2;
pub mod v26_2;
pub mod v26_3;

use std::io;

use enum_dispatch::enum_dispatch;
use flate2::{Decompress, FlushDecompress};
use mc_protocol::{packet::RawPacket, ser::Deserialize, varint::VarInt};

use crate::events::ClientId;

#[enum_dispatch(Version)]
pub trait VersionProtocol {
    fn handle_c2s(
        &mut self,
        packet: &RawPacket,
        client_id: ClientId,
        is_active: bool,
        both_online: bool,
    ) -> Option<ServerBoundEvent>;
    fn handle_s2c(&mut self, packet: &RawPacket, both_online: bool) -> Option<ClientBoundEvent>;
    fn update_threshold(&mut self, threshold: Option<i32>);
    fn handle_client_disconnect(&mut self, new_active: ClientId) -> Option<ClientDisconnectEvent>;
    fn parse_login_start(&self, packet: &RawPacket) -> Option<String>;
}

pub enum ServerBoundEvent {
    SendToInactive(RawPacket),
    SkipRelay,
    /// The client acked a protocol phase switch and can take packets of the new phase.
    PhaseAcked,
    /// The new active client acked a phase switch the server already got from the old one.
    PhaseCaughtUp,
}

pub enum ClientBoundEvent {
    /// Starts a new protocol phase; a client must ack it before getting further packets.
    PhaseSwitch,
}

pub enum ClientDisconnectEvent {
    SendToServer(Vec<RawPacket>),
}

#[enum_dispatch]
pub enum Version {
    V1_16_5(v1_16_5::VersionData),
    V1_17(v1_17::VersionData),
    V1_17_1(v1_17_1::VersionData),
    V1_18_1(v1_18_1::VersionData),
    V1_18_2(v1_18_2::VersionData),
    V1_19(v1_19::VersionData),
    V1_19_2(v1_19_2::VersionData),
    V1_19_3(v1_19_3::VersionData),
    V1_19_4(v1_19_4::VersionData),
    V1_20_1(v1_20_1::VersionData),
    V1_20_2(v1_20_2::VersionData),
    V1_20_4(v1_20_4::VersionData),
    V1_20_6(v1_20_6::VersionData),
    V1_21_1(v1_21_1::VersionData),
    V1_21_3(v1_21_3::VersionData),
    V1_21_4(v1_21_4::VersionData),
    V1_21_5(v1_21_5::VersionData),
    V1_21_6(v1_21_6::VersionData),
    V1_21_8(v1_21_8::VersionData),
    V1_21_10(v1_21_10::VersionData),
    V1_21_11(v1_21_11::VersionData),
    V26_1_2(v26_1_2::VersionData),
    V26_2(v26_2::VersionData),
    V26_3(v26_3::VersionData),
}

impl Version {
    /// Construct the appropriate `Version` for the given protocol number.
    /// Returns `None` if the protocol is not supported.
    pub fn from_protocol(protocol: i32) -> Option<Self> {
        match protocol {
            754 => Some(Version::V1_16_5(v1_16_5::VersionData::new())),
            755 => Some(Version::V1_17(v1_17::VersionData::new())),
            756 => Some(Version::V1_17_1(v1_17_1::VersionData::new())),
            757 => Some(Version::V1_18_1(v1_18_1::VersionData::new())),
            758 => Some(Version::V1_18_2(v1_18_2::VersionData::new())),
            759 => Some(Version::V1_19(v1_19::VersionData::new())),
            760 => Some(Version::V1_19_2(v1_19_2::VersionData::new())),
            761 => Some(Version::V1_19_3(v1_19_3::VersionData::new())),
            762 => Some(Version::V1_19_4(v1_19_4::VersionData::new())),
            763 => Some(Version::V1_20_1(v1_20_1::VersionData::new())),
            764 => Some(Version::V1_20_2(v1_20_2::VersionData::new())),
            765 => Some(Version::V1_20_4(v1_20_4::VersionData::new())),
            766 => Some(Version::V1_20_6(v1_20_6::VersionData::new())),
            767 => Some(Version::V1_21_1(v1_21_1::VersionData::new())),
            768 => Some(Version::V1_21_3(v1_21_3::VersionData::new())),
            769 => Some(Version::V1_21_4(v1_21_4::VersionData::new())),
            770 => Some(Version::V1_21_5(v1_21_5::VersionData::new())),
            771 => Some(Version::V1_21_6(v1_21_6::VersionData::new())),
            772 => Some(Version::V1_21_8(v1_21_8::VersionData::new())),
            773 => Some(Version::V1_21_10(v1_21_10::VersionData::new())),
            774 => Some(Version::V1_21_11(v1_21_11::VersionData::new())),
            775 => Some(Version::V26_1_2(v26_1_2::VersionData::new())),
            776 => Some(Version::V26_2(v26_2::VersionData::new())),
            777 => Some(Version::V26_3(v26_3::VersionData::new())),
            _ => None,
        }
    }
}

/// Reads packet ids without inflating the whole frame, reusing one inflate state.
pub struct PacketIdReader(Decompress);

impl Default for PacketIdReader {
    fn default() -> Self {
        Self(Decompress::new(true))
    }
}

impl PacketIdReader {
    pub fn read(&mut self, packet: &RawPacket, threshold: Option<i32>) -> io::Result<i32> {
        let mut data = packet.data.as_slice();
        if threshold.is_some_and(|t| t >= 0) && read_varint(&mut data)? != 0 {
            self.0.reset(true);
            let mut head = [0u8; 5];
            // Finish on a fresh stream inflates straight into `head`, skipping the 32 KiB window.
            self.0
                .decompress(data, &mut head, FlushDecompress::Finish)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            return read_varint(&mut &head[..self.0.total_out() as usize]);
        }
        read_varint(&mut data)
    }
}

fn read_varint<R: io::Read>(reader: &mut R) -> io::Result<i32> {
    VarInt::read_sync(reader)
        .map(|v| v.0)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

pub fn decode<T: Deserialize>(packet: &RawPacket, threshold: Option<i32>) -> anyhow::Result<T> {
    Ok(packet.uncompress(threshold)?.deserialize_payload()?)
}

/// Shared tests for the 1.21.x modules, which differ only in packet ids.
#[cfg(test)]
macro_rules! config_phase_tests {
    () => {
        use mc_protocol::{packet::PacketId, ser::Serialize};

        use super::*;
        use crate::controller::tests::{C2s, Phase, S2c, Spot, Wire, pack, unpack};

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
            assert_eq!((got.delta_x, got.delta_y, got.delta_z), (0.0, 0.0, 0.0));
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
            let close = c2s::game::ContainerClose {
                container_id: VarInt(3),
            };
            let Some(ServerBoundEvent::SendToInactive(packet)) = c2s(&mut data, &close, P, P, true)
            else {
                panic!("container close was not mirrored");
            };
            let got: s2c::game::ContainerClose = super::super::decode(&packet, None).unwrap();
            assert_eq!(got.container_id.0, 3);
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

            let Some(ClientDisconnectEvent::SendToServer(packets)) =
                data.handle_client_disconnect(S)
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

        impl Wire for VersionData {
            const CONFIGURATION: bool = true;

            fn write_c2s(phase: Phase, packet: C2s) -> UncompressedPacket {
                match packet {
                    C2s::LoginAck => pack(&c2s::login::LoginAcknowledged {}),
                    C2s::FinishConfiguration => pack(&c2s::configuration::FinishConfiguration {}),
                    C2s::ConfigurationAck => pack(&c2s::game::ConfigurationAcknowledged {}),
                    C2s::Teleport(id, _) => pack(&teleport_ack(id)),
                    C2s::Pos(x, y, z) => pack(&c2s::game::Pos {
                        x,
                        y,
                        z,
                        ..Default::default()
                    }),
                    C2s::Rot(yaw, pitch) => pack(&rot(yaw, pitch)),
                    C2s::PosRot(at) => pack(&c2s::game::PosRot {
                        x: at.x,
                        y: at.y,
                        z: at.z,
                        yaw: at.yaw,
                        pitch: at.pitch,
                        ..Default::default()
                    }),
                    C2s::Pong(id) => pack(&c2s::game::Pong { id }),
                    C2s::KeepAlive(id) if phase == Phase::Configuration => {
                        pack(&c2s::configuration::KeepAlive { id })
                    }
                    C2s::KeepAlive(id) => pack(&c2s::game::KeepAlive { id }),
                    C2s::ContainerClose(id) => pack(&c2s::game::ContainerClose {
                        container_id: VarInt(id),
                    }),
                    C2s::Raw(id) => UncompressedPacket::new(id, vec![]),
                }
            }

            fn write_s2c(phase: Phase, packet: S2c) -> UncompressedPacket {
                match packet {
                    S2c::Position(id, at) => pack(&s2c::game::Position {
                        id: VarInt(id),
                        x: at.x,
                        y: at.y,
                        z: at.z,
                        delta_x: 0.0,
                        delta_y: 0.0,
                        delta_z: 0.0,
                        yaw: at.yaw,
                        pitch: at.pitch,
                        relative_flags: 0,
                    }),
                    S2c::Ping(id) => pack(&s2c::game::Ping { id }),
                    S2c::KeepAlive(id) if phase == Phase::Configuration => {
                        pack(&s2c::configuration::KeepAlive { id })
                    }
                    S2c::KeepAlive(id) => pack(&s2c::game::KeepAlive { id }),
                    S2c::ContainerClose(id) => pack(&s2c::game::ContainerClose {
                        container_id: VarInt(id),
                    }),
                    S2c::StartConfiguration => pack(&s2c::game::StartConfiguration {}),
                    S2c::FinishConfiguration => pack(&s2c::configuration::FinishConfiguration {}),
                    S2c::Raw(id, len) => UncompressedPacket::new(id, vec![0; len]),
                }
            }

            fn read_c2s(phase: Phase, packet: &UncompressedPacket) -> Option<C2s> {
                use c2s::{configuration, game, login};
                Some(match (phase, packet.packet_id) {
                    (Phase::Login, login::LoginAcknowledged::PACKET_ID) => C2s::LoginAck,
                    (Phase::Configuration, configuration::FinishConfiguration::PACKET_ID) => {
                        C2s::FinishConfiguration
                    }
                    (Phase::Game, game::ConfigurationAcknowledged::PACKET_ID) => {
                        C2s::ConfigurationAck
                    }
                    (Phase::Game, game::AcceptTeleportation::PACKET_ID) => C2s::Teleport(
                        unpack::<game::AcceptTeleportation>(packet).id.0,
                        Spot::default(),
                    ),
                    (Phase::Game, game::Pos::PACKET_ID) => {
                        let p: game::Pos = unpack(packet);
                        C2s::Pos(p.x, p.y, p.z)
                    }
                    (Phase::Game, game::Rot::PACKET_ID) => {
                        let p: game::Rot = unpack(packet);
                        C2s::Rot(p.yaw, p.pitch)
                    }
                    (Phase::Game, game::PosRot::PACKET_ID) => {
                        let p: game::PosRot = unpack(packet);
                        C2s::PosRot(Spot {
                            x: p.x,
                            y: p.y,
                            z: p.z,
                            yaw: p.yaw,
                            pitch: p.pitch,
                        })
                    }
                    (Phase::Game, game::Pong::PACKET_ID) => {
                        C2s::Pong(unpack::<game::Pong>(packet).id)
                    }
                    (Phase::Configuration, configuration::KeepAlive::PACKET_ID) => {
                        C2s::KeepAlive(unpack::<configuration::KeepAlive>(packet).id)
                    }
                    (Phase::Game, game::KeepAlive::PACKET_ID) => {
                        C2s::KeepAlive(unpack::<game::KeepAlive>(packet).id)
                    }
                    (Phase::Game, game::ContainerClose::PACKET_ID) => {
                        C2s::ContainerClose(unpack::<game::ContainerClose>(packet).container_id.0)
                    }
                    _ => return None,
                })
            }

            fn read_s2c(phase: Phase, packet: &UncompressedPacket) -> Option<S2c> {
                use s2c::{configuration, game};
                Some(match (phase, packet.packet_id) {
                    (Phase::Configuration, configuration::FinishConfiguration::PACKET_ID) => {
                        S2c::FinishConfiguration
                    }
                    (Phase::Game, game::StartConfiguration::PACKET_ID) => S2c::StartConfiguration,
                    (Phase::Game, game::Position::PACKET_ID) => {
                        let p: game::Position = unpack(packet);
                        assert_eq!(p.relative_flags, 0);
                        assert_eq!((p.delta_x, p.delta_y, p.delta_z), (0.0, 0.0, 0.0));
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
                    (Phase::Game, game::Ping::PACKET_ID) => {
                        S2c::Ping(unpack::<game::Ping>(packet).id)
                    }
                    (Phase::Configuration, configuration::KeepAlive::PACKET_ID) => {
                        S2c::KeepAlive(unpack::<configuration::KeepAlive>(packet).id)
                    }
                    (Phase::Game, game::KeepAlive::PACKET_ID) => {
                        S2c::KeepAlive(unpack::<game::KeepAlive>(packet).id)
                    }
                    (Phase::Game, game::ContainerClose::PACKET_ID) => {
                        S2c::ContainerClose(unpack::<game::ContainerClose>(packet).container_id.0)
                    }
                    _ => return None,
                })
            }
        }

        crate::controller::tests::controller_tests!(configuration);
    };
}
#[cfg(test)]
use config_phase_tests;

#[cfg(test)]
mod tests {
    use mc_protocol::packet::UncompressedPacket;

    use super::*;

    #[test]
    fn packet_id_matches_full_decode() {
        let packet = UncompressedPacket::new(0x2c, vec![7; 4096]);
        let mut ids = PacketIdReader::default();
        for threshold in [None, Some(-1), Some(0), Some(256), Some(1_000_000)] {
            let raw = packet.to_raw_packet_compressed(threshold).unwrap();
            assert_eq!(ids.read(&raw, threshold).unwrap(), 0x2c, "{threshold:?}");
            assert_eq!(raw.uncompress(threshold).unwrap().packet_id, 0x2c);
        }
    }

    #[test]
    fn packet_id_reader_reuses_state() {
        let mut ids = PacketIdReader::default();
        let threshold = Some(64);
        for (id, len) in [
            (0x27, 100_000),
            (0x0, 0),
            (300, 200),
            (0x2c, 64),
            (i32::MAX, 100),
        ] {
            let raw = UncompressedPacket::new(id, vec![id as u8; len])
                .to_raw_packet_compressed(threshold)
                .unwrap();
            assert_eq!(ids.read(&raw, threshold).unwrap(), id, "{id} {len}");
        }
    }
}
