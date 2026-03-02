pub mod v1_16_5;
pub mod v1_21_4;

use mc_protocol::packet::{RawPacket, UncompressedPacket};

use crate::controller::ClientId;

// ─── VersionProtocol trait ───────────────────────────────────────────────────

/// Interface every version-specific protocol implementation must satisfy.
///
/// All methods receive an already-decompressed `UncompressedPacket` and return
/// `None` when the packet is not relevant to that method's concern.
pub trait VersionProtocol {
    /// If `packet` is a c2s movement packet (Look / Position / PositionLook):
    /// update stored position and return an encoded s2c position sync packet
    /// ready to forward to the inactive client. Returns `None` otherwise.
    fn try_sync_position(
        &mut self,
        packet: &UncompressedPacket,
        threshold: Option<i32>,
    ) -> Option<RawPacket>;

    /// If `packet` is a c2s Transaction acknowledgement: manage the pending
    /// transaction queue. Returns:
    ///   `None`        — not a c2s Transaction packet
    ///   `Some(false)` — processed normally; relay to server proceeds
    ///   `Some(true)`  — packet consumed; caller must skip server relay
    fn try_handle_c2s_pong(
        &mut self,
        packet: &UncompressedPacket,
        client_id: ClientId,
        both_active: bool,
    ) -> Option<bool>;

    /// If `packet` is an s2c Transaction from the server: push a new
    /// `TransactionSync` entry and return `Some(action)`. Returns `None`
    /// for all other packet IDs.
    fn try_track_s2c_ping(&mut self, packet: &UncompressedPacket);

    /// Called when the active client switches. Collects encoded c2s Transaction
    /// acks for every pending entry that `new_active` already acknowledged,
    /// drops the rest, and returns the packets to relay to the server.
    fn collect_replay(&mut self, new_active: ClientId, threshold: Option<i32>) -> Vec<RawPacket>;
}

// ─── Version enum ─────────────────────────────────────────────────────────────

/// All supported Minecraft protocol versions.
///
/// # Adding a new version
/// 1. Create `src/protocols/vX_Y_Z/` with `packets.rs` and `mod.rs`
/// 2. Add `pub mod vX_Y_Z;` here
/// 3. Implement `VersionProtocol` for the new `VersionData`
/// 4. Add a variant to this enum
/// 5. Add an arm to `from_protocol()`
/// 6. Add arms to the four dispatch `match` blocks below
///
/// `main.rs` and `controller.rs` require **zero changes**.
pub enum Version {
    V1_16_5(v1_16_5::VersionData),
    V1_21_4(v1_21_4::VersionData),
}

impl Version {
    /// Construct the appropriate `Version` for the given protocol number.
    /// Returns `None` if the protocol is not supported.
    pub fn from_protocol(protocol: i32) -> Option<Self> {
        match protocol {
            754 => Some(Version::V1_16_5(v1_16_5::VersionData::new())),
            769 => Some(Version::V1_21_4(v1_21_4::VersionData::new())),
            _ => None,
        }
    }

    pub fn try_sync_position(
        &mut self,
        packet: &UncompressedPacket,
        threshold: Option<i32>,
    ) -> Option<RawPacket> {
        match self {
            Version::V1_16_5(d) => d.try_sync_position(packet, threshold),
            Version::V1_21_4(d) => d.try_sync_position(packet, threshold),
        }
    }

    pub fn try_handle_c2s_transaction(
        &mut self,
        packet: &UncompressedPacket,
        client_id: ClientId,
        both_active: bool,
    ) -> Option<bool> {
        match self {
            Version::V1_16_5(d) => d.try_handle_c2s_pong(packet, client_id, both_active),
            Version::V1_21_4(d) => d.try_handle_c2s_pong(packet, client_id, both_active),
        }
    }

    pub fn try_track_s2c_transaction(&mut self, packet: &UncompressedPacket) {
        match self {
            Version::V1_16_5(d) => d.try_track_s2c_ping(packet),
            Version::V1_21_4(d) => d.try_track_s2c_ping(packet),
        }
    }

    pub fn collect_replay(
        &mut self,
        new_active: ClientId,
        threshold: Option<i32>,
    ) -> Vec<RawPacket> {
        match self {
            Version::V1_16_5(d) => d.collect_replay(new_active, threshold),
            Version::V1_21_4(d) => d.collect_replay(new_active, threshold),
        }
    }
}
