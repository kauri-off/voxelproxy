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
pub mod v1_21_11;
pub mod v1_21_4;
pub mod v1_21_8;

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
        both_active: bool,
    ) -> Option<ServerBoundEvent>;
    fn handle_s2c(&mut self, packet: &RawPacket, both_active: bool) -> Option<ClientBoundEvent>;
    fn update_threshold(&mut self, threshold: Option<i32>);
    fn handle_client_disconnect(&mut self, new_active: ClientId) -> Option<ClientDisconnectEvent>;
    fn parse_login_start(&self, packet: &RawPacket) -> Option<String>;
}

pub enum ServerBoundEvent {
    SendToInactive(RawPacket),
    SkipRelay,
    /// The client acked a protocol phase switch and can take packets of the new phase.
    PhaseAcked,
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
    V1_21_4(v1_21_4::VersionData),
    V1_21_8(v1_21_8::VersionData),
    V1_21_11(v1_21_11::VersionData),
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
            769 => Some(Version::V1_21_4(v1_21_4::VersionData::new())),
            772 => Some(Version::V1_21_8(v1_21_8::VersionData::new())),
            774 => Some(Version::V1_21_11(v1_21_11::VersionData::new())),
            _ => None,
        }
    }

    pub fn supported_versions() -> &'static [&'static str] {
        &[
            v1_16_5::VersionData::VERSION,
            v1_17::VersionData::VERSION,
            v1_17_1::VersionData::VERSION,
            v1_18_1::VersionData::VERSION,
            v1_18_2::VersionData::VERSION,
            v1_19::VersionData::VERSION,
            v1_19_2::VersionData::VERSION,
            v1_19_3::VersionData::VERSION,
            v1_19_4::VersionData::VERSION,
            v1_20_1::VersionData::VERSION,
            v1_20_2::VersionData::VERSION,
            v1_20_4::VersionData::VERSION,
            v1_20_6::VersionData::VERSION,
            v1_21_1::VersionData::VERSION,
            v1_21_4::VersionData::VERSION,
            v1_21_8::VersionData::VERSION,
            v1_21_11::VersionData::VERSION,
        ]
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
            both_active: bool,
        ) -> Option<ServerBoundEvent> {
            let packet = raw(data, packet);
            data.handle_c2s(&packet, client, client == active, both_active)
        }

        fn s2c<T: PacketId + Serialize>(
            data: &mut VersionData,
            packet: &T,
            both_active: bool,
        ) -> Option<ClientBoundEvent> {
            let packet = raw(data, packet);
            data.handle_s2c(&packet, both_active)
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
        fn own_teleport_ack_is_skipped_only_without_peer() {
            let mut data = in_game(None);
            let ours = c2s::game::AcceptTeleportation {
                id: VarInt(TELEPORT_ID),
            };
            let servers = c2s::game::AcceptTeleportation { id: VarInt(1) };
            assert!(c2s(&mut data, &ours, P, P, true).is_none());
            assert!(c2s(&mut data, &servers, S, S, false).is_none());
            assert!(matches!(
                c2s(&mut data, &ours, S, S, false),
                Some(ServerBoundEvent::SkipRelay)
            ));
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
