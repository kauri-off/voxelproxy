pub mod packets;

use mc_protocol::{
    packet::{RawPacket, UncompressedPacket},
    varint::VarInt,
};

use super::VersionProtocol;
use crate::controller::ClientId;
use packets::{c2s, s2c};

// ─── TransactionSync ────────────────────────────────────────────────────────

/// Tracks whether each client has acknowledged a single server-initiated
/// transaction. Version-agnostic: only the packet IDs and encoding differ
/// per version, which is handled inside each `VersionData`.
pub struct TransactionSync {
    pub action: i16,
    cheat_sent: bool,
    legit_sent: bool,
}

impl TransactionSync {
    /// Creates a tracker for a transaction, with both clients unconfirmed.
    pub fn new(action: i16) -> Self {
        Self {
            action,
            cheat_sent: false,
            legit_sent: false,
        }
    }

    /// Marks `client` as having confirmed this transaction.
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

/// Version-specific state for Minecraft 1.16.5 (protocol 763).
/// Owns both the authoritative player position and the pending transaction queue.
pub struct VersionData {
    pub position: s2c::Position,
    pub transactions: Vec<TransactionSync>,
}

impl VersionData {
    pub fn new() -> Self {
        Self {
            position: s2c::Position {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                yaw: 0.0,
                pitch: 0.0,
                flags: 0,
                teleport_id: VarInt(0),
            },
            transactions: vec![],
        }
    }

    /// Update `self.position` from a c2s movement packet.
    fn update_position(&mut self, packet: &UncompressedPacket) -> anyhow::Result<()> {
        match packet.packet_id {
            c2s::Position::PACKET_ID => {
                let p: c2s::Position = packet.deserialize_payload()?;
                self.position.x = p.x;
                self.position.y = p.y;
                self.position.z = p.z;
            }
            c2s::PositionLook::PACKET_ID => {
                let p: c2s::PositionLook = packet.deserialize_payload()?;
                self.position.x = p.x;
                self.position.y = p.y;
                self.position.z = p.z;
                self.position.yaw = p.yaw;
                self.position.pitch = p.pitch;
            }
            c2s::Look::PACKET_ID => {
                let p: c2s::Look = packet.deserialize_payload()?;
                self.position.yaw = p.yaw;
                self.position.pitch = p.pitch;
            }
            _ => {}
        }
        Ok(())
    }
}

impl VersionProtocol for VersionData {
    fn try_sync_position(
        &mut self,
        packet: &UncompressedPacket,
        threshold: Option<i32>,
    ) -> Option<RawPacket> {
        match packet.packet_id {
            c2s::Look::PACKET_ID | c2s::Position::PACKET_ID | c2s::PositionLook::PACKET_ID => {
                let _ = self.update_position(packet);
                Some(
                    UncompressedPacket::from_packet(&self.position)
                        .unwrap()
                        .to_raw_packet_compressed(threshold)
                        .unwrap(),
                )
            }
            _ => None,
        }
    }

    fn try_handle_c2s_pong(
        &mut self,
        packet: &UncompressedPacket,
        client_id: ClientId,
        both_active: bool,
    ) -> Option<bool> {
        if packet.packet_id != c2s::Transaction::PACKET_ID {
            return None;
        }

        let t: c2s::Transaction = packet.deserialize_payload().unwrap();

        if both_active {
            // Both clients connected: remove entry once both have acknowledged it.
            if let Some(i) = self
                .transactions
                .iter_mut()
                .position(|s| s.action == t.action && s.sent(client_id))
            {
                self.transactions.remove(i);
            }
            Some(false)
        } else {
            // One client gone: skip relay if the absent client already acked this.
            if let Some(head) = self.transactions.get(0) {
                if head.is_sent(client_id.opposite()) {
                    println!("Синхронизация: Пропуск: {}", head.action);
                    self.transactions.remove(0);
                    return Some(true); // caller must `continue` (skip relay to server)
                }
            }
            Some(false)
        }
    }

    fn try_track_s2c_ping(&mut self, packet: &UncompressedPacket) {
        if packet.packet_id != s2c::Transaction::PACKET_ID {
            return;
        }

        let t: s2c::Transaction = packet.deserialize_payload().unwrap();
        self.transactions.push(TransactionSync::new(t.action));
    }

    fn collect_replay(&mut self, new_active: ClientId, threshold: Option<i32>) -> Vec<RawPacket> {
        let mut to_send = vec![];
        self.transactions.retain(|t| {
            if t.is_sent(new_active) {
                to_send.push(t.action);
                true
            } else {
                false
            }
        });

        to_send
            .into_iter()
            .map(|action| {
                println!("Синхронизация: Отправка: {}", action);
                let tx = c2s::Transaction {
                    window_id: 0,
                    action,
                    accepted: true,
                };
                UncompressedPacket::from_packet(&tx)
                    .unwrap()
                    .to_raw_packet_compressed(threshold)
                    .unwrap()
            })
            .collect()
    }
}
