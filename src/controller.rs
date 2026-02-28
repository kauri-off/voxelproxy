use mc_protocol::{
    packet::{RawPacket, UncompressedPacket},
    varint::VarInt,
};
use tokio::{
    net::tcp::{OwnedReadHalf, OwnedWriteHalf},
    sync::mpsc::{Receiver, Sender},
};

use crate::packets::v1_16_5::{c2s, s2c};

/// Identifies which of the two connected clients a packet or event belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientId {
    Cheat,
    Legit,
}

impl ClientId {
    /// Returns the other client variant.
    fn opposite(&self) -> ClientId {
        match self {
            ClientId::Cheat => ClientId::Legit,
            ClientId::Legit => ClientId::Cheat,
        }
    }
}

/// Events sent to the Controller from the background I/O tasks.
#[derive(Debug)]
pub enum Event {
    /// A packet was received from a client.
    ClientData(ClientId, RawPacket),
    /// A client's TCP connection was closed or errored.
    ClientDisconnected(ClientId),
    /// A packet was received from the upstream server.
    ServerData(RawPacket),
}

/// Tracks whether each client has acknowledged a single server-initiated transaction.
///
/// The server sends a transaction to both clients; both must confirm it before the
/// entry is removed from the pending queue.
pub struct TransactionSync {
    /// The action ID that uniquely identifies this transaction.
    action: i16,
    /// Whether the Cheat client has sent its confirmation.
    cheat_sent: bool,
    /// Whether the Legit client has sent its confirmation.
    legit_sent: bool,
}

impl TransactionSync {
    /// Creates a new tracker for a transaction received from the server.
    /// Both clients start as unconfirmed.
    fn new(transaction: &s2c::Transaction) -> Self {
        Self {
            action: transaction.action,
            cheat_sent: false,
            legit_sent: false,
        }
    }

    /// Marks `client` as having confirmed this transaction.
    /// Returns `true` once both clients have confirmed (i.e. ready to remove from queue).
    fn sent(&mut self, client: ClientId) -> bool {
        match client {
            ClientId::Cheat => self.cheat_sent = true,
            ClientId::Legit => self.legit_sent = true,
        };
        self.cheat_sent == self.legit_sent
    }

    /// Returns whether `client` has already sent its confirmation (non-mutating).
    fn is_sent(&self, client: ClientId) -> bool {
        match client {
            ClientId::Cheat => self.cheat_sent,
            ClientId::Legit => self.legit_sent,
        }
    }
}

/// Central coordinator that routes packets between two clients and the upstream server,
/// and keeps position and transaction state synchronised across both clients.
pub struct Controller {
    /// Which client is currently the authoritative sender to the server.
    active_client: ClientId,
    /// Channel for sending packets to the Cheat client.
    cheat_tx: Sender<RawPacket>,
    /// Channel for sending packets to the Legit client.
    legit_tx: Sender<RawPacket>,
    /// Channel for sending packets to the upstream server.
    remote_tx: Sender<RawPacket>,
    /// Receives events (client data, disconnections, server data) from I/O tasks.
    event_rx: Receiver<Event>,
    /// Compression threshold negotiated during login; `None` means no compression.
    threshold: Option<i32>,
    /// Whether the Cheat client is still connected.
    cheat_active: bool,
    /// Whether the Legit client is still connected.
    legit_active: bool,
    /// The last known authoritative player position/rotation (driven by the active client).
    position: s2c::Position,
    /// Pending transactions waiting for acknowledgement from both clients.
    transactions: Vec<TransactionSync>,
}

impl Controller {
    /// Constructs a new Controller. Both clients are assumed to be active on creation.
    /// Player position defaults to the world origin (0, 0, 0) with zero rotation.
    pub fn new(
        active_client: ClientId,
        cheat_tx: Sender<RawPacket>,
        legit_tx: Sender<RawPacket>,
        remote_tx: Sender<RawPacket>,
        event_rx: Receiver<Event>,
        threshold: Option<i32>,
    ) -> Self {
        Self {
            active_client,
            cheat_tx,
            legit_tx,
            remote_tx,
            event_rx,
            threshold,
            cheat_active: true,
            legit_active: true,
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
    /// Main event loop. Runs until the channel closes (both I/O tasks have exited).
    ///
    /// Each iteration handles one of three event types:
    /// - `ClientData`       — position sync, transaction tracking, relay to server
    /// - `ClientDisconnected` — update state, optionally switch active client & replay transactions
    /// - `ServerData`       — track new transactions, broadcast to active clients
    pub async fn run(mut self) {
        while let Some(event) = self.event_rx.recv().await {
            match event {
                Event::ClientData(client_id, packet) => {
                    // ── Position sync ────────────────────────────────────────────────────
                    // Only the active client drives the authoritative position. When both
                    // clients are connected we forward a synthetic s2c::Position packet to
                    // the inactive client so its state stays in sync with the active one.
                    if client_id == self.active_client {
                        // Position Sync
                        if self.both_active() {
                            if let Ok(packet) = packet.uncompress(self.threshold) {
                                match packet.packet_id {
                                    c2s::Look::PACKET_ID
                                    | c2s::Position::PACKET_ID
                                    | c2s::PositionLook::PACKET_ID => {
                                        let _ = self.update_position(&packet);
                                        let notice =
                                            UncompressedPacket::from_packet(&self.position)
                                                .unwrap()
                                                .to_raw_packet_compressed(self.threshold)
                                                .unwrap();

                                        match self.active_client.opposite() {
                                            ClientId::Cheat => {
                                                self.cheat_tx.send(notice).await.ok()
                                            }
                                            ClientId::Legit => {
                                                self.legit_tx.send(notice).await.ok()
                                            }
                                        };
                                    }

                                    _ => {}
                                }
                            }
                        }
                    }

                    // ── Transaction tracking ─────────────────────────────────────────────
                    if let Ok(packet) = packet.uncompress(self.threshold) {
                        if packet.packet_id == c2s::Transaction::PACKET_ID {
                            let t: c2s::Transaction = packet.deserialize_payload().unwrap();

                            if self.both_active() {
                                // Both clients are connected: a transaction is complete only
                                // when *both* have acknowledged it. `sent()` marks this client
                                // and returns `true` the moment the second ack arrives, at
                                // which point the entry is removed from the pending queue.
                                if let Some(index) =
                                    self.transactions.iter_mut().position(|sync_packet| {
                                        sync_packet.action == t.action
                                            && sync_packet.sent(client_id)
                                    })
                                {
                                    self.transactions.remove(index);
                                }
                            } else {
                                // One client is gone. If the head of the queue was already
                                // acknowledged by the now-absent client, skip (discard) it so
                                // a stale entry doesn't block the still-active client.
                                if let Some(t) = self.transactions.get(0) {
                                    if t.is_sent(client_id.opposite()) {
                                        println!("Синхронизация: Пропуск: {}", t.action);
                                        self.transactions.remove(0);
                                        continue;
                                    }
                                }
                            }
                        }
                    }

                    // ── Server relay ─────────────────────────────────────────────────────
                    // Only the active client's packets are forwarded to the server.
                    if client_id == self.active_client {
                        if let Err(e) = self.remote_tx.send(packet).await {
                            println!("Ошибка отправки пакета на сервер: {}", e);
                            return;
                        }
                    }
                }
                Event::ClientDisconnected(client_id) => {
                    // If both_active() is already false here, the *second* client just
                    // disconnected — nothing left to do, shut down the controller.
                    if !self.both_active() {
                        println!("Оба клиента отключились");
                        return;
                    }
                    // Mark the disconnecting client as inactive.
                    match client_id {
                        ClientId::Cheat => self.cheat_active = false,
                        ClientId::Legit => self.legit_active = false,
                    };

                    if self.active_client == client_id {
                        // The active client disconnected — switch control to the other client.
                        self.active_client = match client_id {
                            ClientId::Cheat => ClientId::Legit,
                            ClientId::Legit => ClientId::Cheat,
                        };
                        println!("Переключился на {:?}", self.active_client);

                        // Transaction replay after client switch:
                        // - For each pending transaction that the new active client already
                        //   acknowledged before the switch, re-send that ack to the server
                        //   now (the old client's ack was never forwarded). `retain` keeps
                        //   those entries (`true`) while collecting them in `to_send`.
                        // - Transactions that the new active client never confirmed are
                        //   silently dropped (`false`), because we have no ack to replay.
                        let mut to_send = vec![];
                        self.transactions.retain(|t| {
                            if t.is_sent(self.active_client) {
                                let transaction = c2s::Transaction {
                                    window_id: 0,
                                    action: t.action,
                                    accepted: true,
                                };
                                to_send.push(transaction);
                                true
                            } else {
                                false
                            }
                        });

                        for transaction in to_send {
                            println!("Синхронизация: Отправка: {}", transaction.action);
                            self.remote_tx
                                .send(
                                    UncompressedPacket::from_packet(&transaction)
                                        .unwrap()
                                        .to_raw_packet_compressed(self.threshold)
                                        .unwrap(),
                                )
                                .await
                                .unwrap();
                        }
                    }
                }
                Event::ServerData(packet) => {
                    // Track every new transaction so we can wait for both client acks.
                    if let Ok(packet) = packet.uncompress(self.threshold) {
                        if packet.packet_id == s2c::Transaction::PACKET_ID {
                            let t: s2c::Transaction = packet.deserialize_payload().unwrap();
                            self.transactions.push(TransactionSync::new(&t));
                        }
                    }
                    // Broadcast the raw packet (compressed or not) to whichever clients
                    // are still active.
                    if self.cheat_active {
                        let _ = self.cheat_tx.send(packet.clone()).await;
                    }
                    if self.legit_active {
                        let _ = self.legit_tx.send(packet).await;
                    }
                }
            }
        }
    }

    /// Decodes a c2s movement packet and updates the controller's authoritative
    /// position/rotation. Only called for Look, Position, and PositionLook packets.
    fn update_position(&mut self, packet: &UncompressedPacket) -> anyhow::Result<()> {
        match packet.packet_id {
            c2s::Position::PACKET_ID => {
                let pos: c2s::Position = packet.deserialize_payload()?;
                self.position.x = pos.x;
                self.position.y = pos.y;
                self.position.z = pos.z;
            }
            c2s::PositionLook::PACKET_ID => {
                let pos: c2s::PositionLook = packet.deserialize_payload()?;
                self.position.x = pos.x;
                self.position.y = pos.y;
                self.position.z = pos.z;
                self.position.yaw = pos.yaw;
                self.position.pitch = pos.pitch;
            }
            c2s::Look::PACKET_ID => {
                let pos: c2s::Look = packet.deserialize_payload()?;
                self.position.yaw = pos.yaw;
                self.position.pitch = pos.pitch;
            }

            _ => {}
        };

        Ok(())
    }

    /// Returns `true` only when both clients are currently connected.
    fn both_active(&self) -> bool {
        (self.cheat_active == self.legit_active) && self.cheat_active
    }
}

/// Drives a single client connection using two concurrent tasks:
/// - **Read task**: reads packets from the TCP socket and sends them to the Controller
///   as `Event::ClientData`; sends `Event::ClientDisconnected` on any read error.
/// - **Write task**: receives packets from the Controller via `packet_rx` and writes
///   them to the TCP socket; exits silently on write error.
pub async fn run_client(
    read_half: OwnedReadHalf,
    write_half: OwnedWriteHalf,
    client_id: ClientId,
    event_tx: Sender<Event>,
    mut packet_rx: Receiver<RawPacket>,
) {
    let (mut client_read, mut client_write) = (read_half, write_half);
    let _ = tokio::join!(
        async move {
            loop {
                match RawPacket::read_async(&mut client_read).await {
                    Ok(packet) => {
                        if event_tx
                            .send(Event::ClientData(client_id, packet))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(_) => {
                        event_tx
                            .send(Event::ClientDisconnected(client_id))
                            .await
                            .ok();
                        break;
                    }
                }
            }
        },
        async move {
            while let Some(packet) = packet_rx.recv().await {
                if packet.write_async(&mut client_write).await.is_err() {
                    break;
                };
            }
        }
    );
}

/// Drives the upstream server connection using two concurrent tasks:
/// - **Read task**: reads packets from the server and sends them to the Controller
///   as `Event::ServerData`; exits on read error.
/// - **Write task**: receives packets from the Controller via `packet_rx` and writes
///   them to the server socket; exits silently on write error.
pub async fn run_server(
    read_half: tokio::net::tcp::OwnedReadHalf,
    write_half: tokio::net::tcp::OwnedWriteHalf,
    event_tx: Sender<Event>,
    mut packet_rx: Receiver<RawPacket>,
) {
    let (mut server_read, mut server_write) = (read_half, write_half);
    let _ = tokio::join!(
        async move {
            loop {
                match RawPacket::read_async(&mut server_read).await {
                    Ok(packet) => {
                        if event_tx.send(Event::ServerData(packet)).await.is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        },
        async move {
            while let Some(packet) = packet_rx.recv().await {
                if packet.write_async(&mut server_write).await.is_err() {
                    break;
                }
            }
        }
    );
}
