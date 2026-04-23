pub mod v1_16_5;
pub mod v1_20_1;
pub mod v1_21_4;
pub mod v1_21_8;

use enum_dispatch::enum_dispatch;
use mc_protocol::packet::RawPacket;
use tauri::AppHandle;

use crate::controller::ClientId;

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
}

pub enum ClientBoundEvent {}

pub enum ClientDisconnectEvent {
    SendToServer(Vec<RawPacket>),
}

#[enum_dispatch]
pub enum Version {
    V1_16_5(v1_16_5::VersionData),
    V1_20_1(v1_20_1::VersionData),
    V1_21_4(v1_21_4::VersionData),
    V1_21_8(v1_21_8::VersionData),
}

impl Version {
    /// Construct the appropriate `Version` for the given protocol number.
    /// Returns `None` if the protocol is not supported.
    pub fn from_protocol(protocol: i32, app: AppHandle) -> Option<Self> {
        match protocol {
            754 => Some(Version::V1_16_5(v1_16_5::VersionData::new(app))),
            763 => Some(Version::V1_20_1(v1_20_1::VersionData::new(app))),
            769 => Some(Version::V1_21_4(v1_21_4::VersionData::new(app))),
            772 => Some(Version::V1_21_8(v1_21_8::VersionData::new(app))),
            _ => None,
        }
    }

    pub fn supported_versions() -> &'static [&'static str] {
        &[
            v1_16_5::VersionData::VERSION,
            v1_20_1::VersionData::VERSION,
            v1_21_4::VersionData::VERSION,
            v1_21_8::VersionData::VERSION,
        ]
    }
}
