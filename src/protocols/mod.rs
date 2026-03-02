pub mod v1_16_5;
pub mod v1_20_1;
pub mod v1_21_4;

use enum_dispatch::enum_dispatch;
use mc_protocol::packet::{RawPacket, UncompressedPacket};

use crate::controller::ClientId;

// ─── PingSync ────────────────────────────────────────────────────────────────

/// Tracks whether each client has acknowledged a single server-initiated ping.
/// Shared by all versions that use Ping/Pong synchronisation (1.20.1+).
/// Version 1.16.5 uses its own `TransactionSync` with an `i16` action field.
pub struct PingSync {
    pub id: i32,
    cheat_sent: bool,
    legit_sent: bool,
}

impl PingSync {
    /// Creates a tracker for a ping, with both clients unconfirmed.
    pub fn new(id: i32) -> Self {
        Self {
            id,
            cheat_sent: false,
            legit_sent: false,
        }
    }

    /// Marks `client` as having confirmed this ping.
    /// Returns `true` once both clients have confirmed (ready to remove).
    pub fn sent(&mut self, client: ClientId) -> bool {
        match client {
            ClientId::Cheat => self.cheat_sent = true,
            ClientId::Legit => self.legit_sent = true,
        }
        self.cheat_sent && self.legit_sent
    }

    /// Returns whether `client` has already sent its confirmation (non-mutating).
    pub fn is_sent(&self, client: ClientId) -> bool {
        match client {
            ClientId::Cheat => self.cheat_sent,
            ClientId::Legit => self.legit_sent,
        }
    }
}

// ─── VersionProtocol trait ───────────────────────────────────────────────────

/// Interface every version-specific protocol implementation must satisfy.
///
/// # Ping synchronisation overview
///
/// The server periodically sends a Ping (or ContainerAck in 1.16.5) to the proxy.
/// The proxy forwards it to **both** connected clients and tracks acknowledgements
/// in a FIFO queue ([`PingSync`] entries). An entry is removed automatically as
/// soon as both clients have acknowledged it.
///
/// When the active client switches, the queue can be in one of three states:
///
/// ## 1. Fully synchronised
/// Both clients have acknowledged every pending ping → the queue is empty → no
/// action needed.
///
/// ## 2. Previous client acknowledged more than the new one
/// The old client sent pongs that the new client has not yet sent. Those entries
/// are still in the queue with the old client's flag set. As the new active client
/// now sends its pongs one by one, [`Self::try_handle_c2s_pong`] checks whether the old
/// client already acked the head entry. If it did, the relay to the server is
/// **skipped** (the server already received that pong from the old session) and
/// the entry is removed.
///
/// ## 3. New client acknowledged more than the previous one (pongs were discarded)
/// While the new client was inactive, it still sent pongs in response to pings —
/// but those pongs were discarded by the proxy because only the active client's
/// traffic is forwarded. The old active client never sent those pongs, so those
/// entries remain in the queue with only the new client's flag set.
///
/// An important invariant: because entries are removed the moment *both* clients
/// acknowledge them, any entry still present in the queue with the new client's
/// flag set **necessarily** means the old client did not ack it. There is no need
/// to check the old client's flag explicitly.
///
/// Without intervention the server would time out waiting for those pongs.
/// [`Self::collect_replay`] detects these entries and immediately replays the
/// corresponding pong packets to the server.
#[enum_dispatch(Version)]
pub trait VersionProtocol {
    /// If `packet` is a c2s movement packet (`Rot` / `Pos` / `PosRot`):
    /// updates the stored position and returns an encoded s2c
    /// `PlayerPosition` packet ready to forward to the inactive client,
    /// keeping it in sync with the active one. Returns `None` otherwise.
    fn try_sync_position(
        &mut self,
        packet: &UncompressedPacket,
        threshold: Option<i32>,
    ) -> Option<RawPacket>;

    /// If `packet` is a c2s sync acknowledgement (Pong or ContainerAck depending
    /// on version), manages the pending sync queue and returns:
    /// - `None`        — not a sync ack; caller processes the packet normally.
    /// - `Some(false)` — sync ack handled; relay to server proceeds.
    /// - `Some(true)`  — sync ack consumed; caller **must skip** the server relay.
    ///
    /// **When `both_active` is `true`** both clients are connected and every ack
    /// is relayed while the queue entry is updated.
    ///
    /// **When `both_active` is `false`** this handles scenario 2 (*previous client
    /// acknowledged more*): if the absent client already acked the head entry, the
    /// current client's matching pong is **not** relayed — the server received it
    /// from the previous session — and the entry is removed from the queue.
    fn try_handle_c2s_pong(
        &mut self,
        packet: &UncompressedPacket,
        client_id: ClientId,
        both_active: bool,
    ) -> Option<bool>;

    /// If `packet` is an s2c sync packet from the server (Ping or ContainerAck
    /// depending on version), appends a new [`PingSync`] entry to the queue.
    /// Does nothing for all other packet IDs.
    fn try_track_s2c_ping(&mut self, packet: &UncompressedPacket);

    /// Called immediately after the active client switches to `new_active`.
    ///
    /// Handles scenario 3 (*new client acknowledged more while inactive*): scans
    /// the queue for entries where `new_active` already sent an ack that was
    /// discarded at the time. Those acks are re-encoded and returned so the caller
    /// can forward them to the server, preventing a timeout.
    ///
    /// Entries where `new_active` has **not** acked are dropped: they either
    /// represent pings where the old client already relayed a pong to the server
    /// (so the server is satisfied), or pings that neither client has acked yet
    /// (the new active client will respond to them naturally).
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
///
/// `main.rs` and `controller.rs` require **zero changes**.
#[enum_dispatch]
pub enum Version {
    V1_16_5(v1_16_5::VersionData),
    V1_20_1(v1_20_1::VersionData),
    V1_21_4(v1_21_4::VersionData),
}

impl Version {
    /// Construct the appropriate `Version` for the given protocol number.
    /// Returns `None` if the protocol is not supported.
    pub fn from_protocol(protocol: i32) -> Option<Self> {
        match protocol {
            754 => Some(Version::V1_16_5(v1_16_5::VersionData::new())),
            763 => Some(Version::V1_20_1(v1_20_1::VersionData::new())),
            769 => Some(Version::V1_21_4(v1_21_4::VersionData::new())),
            _ => None,
        }
    }
}
