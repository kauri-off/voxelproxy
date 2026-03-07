use mc_protocol::packet::RawPacket;
use tokio::{
    net::tcp::{OwnedReadHalf, OwnedWriteHalf},
    sync::mpsc::{Receiver, Sender},
};

use crate::{
    logger::Logger,
    protocols::{ServerBoundEvent, Version, VersionProtocol},
};

/// Identifies which of the two connected clients a packet or event belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientId {
    Primary,
    Secondary,
}

impl ClientId {
    /// Returns the other client variant.
    pub fn opposite(&self) -> ClientId {
        match self {
            ClientId::Primary => ClientId::Secondary,
            ClientId::Secondary => ClientId::Primary,
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

/// Central coordinator that routes packets between two clients and the upstream
/// server. All version-specific logic (position tracking, ping queue) is
/// delegated to the `Version` field, keeping this struct protocol-agnostic.
pub struct Controller {
    /// Which client is currently the authoritative sender to the server.
    active_client: ClientId,
    /// Channel for sending packets to the Primary client.
    primary_tx: Sender<RawPacket>,
    /// Channel for sending packets to the Secondary client.
    secondary_tx: Sender<RawPacket>,
    /// Channel for sending packets to the upstream server.
    remote_tx: Sender<RawPacket>,
    /// Receives events (client data, disconnections, server data) from I/O tasks.
    event_rx: Receiver<Event>,
    /// Whether the Primary client is still connected.
    primary_active: bool,
    /// Whether the Secondary client is still connected.
    secondary_active: bool,
    /// Version-specific state: position tracking and sync-packet queue.
    version: Version,
    /// Logger for emitting status events to the GUI.
    log: Logger,
}

impl Controller {
    /// Constructs a new Controller. Both clients are assumed to be active on creation.
    pub fn new(
        active_client: ClientId,
        primary_tx: Sender<RawPacket>,
        secondary_tx: Sender<RawPacket>,
        remote_tx: Sender<RawPacket>,
        event_rx: Receiver<Event>,
        version: Version,
        log: Logger,
    ) -> Self {
        Self {
            active_client,
            primary_tx,
            secondary_tx,
            remote_tx,
            event_rx,
            primary_active: true,
            secondary_active: true,
            version,
            log,
        }
    }

    /// Main event loop. Runs until the channel closes (both I/O tasks have exited).
    ///
    /// Each iteration handles one of three event types:
    /// - `ClientData`         — position sync, ping tracking, relay to server
    /// - `ClientDisconnected` — update state, optionally switch active client & replay sync packets
    /// - `ServerData`         — track new pings, broadcast to active clients
    pub async fn run(mut self) {
        while let Some(event) = self.event_rx.recv().await {
            match event {
                Event::ClientData(client_id, packet) => {
                    let event = self.version.handle_c2s(
                        &packet,
                        client_id,
                        client_id == self.active_client,
                        self.both_active(),
                    );

                    let mut skip = false;

                    if let Some(event) = event {
                        match event {
                            ServerBoundEvent::SendToInactive(raw_packet) => {
                                if self.both_active() {
                                    match self.active_client.opposite() {
                                        ClientId::Primary => {
                                            self.primary_tx.send(raw_packet).await.ok();
                                        }
                                        ClientId::Secondary => {
                                            self.secondary_tx.send(raw_packet).await.ok();
                                        }
                                    }
                                }
                            }
                            ServerBoundEvent::SkipRelay => skip = true,
                        }
                    }

                    if skip {
                        continue;
                    }

                    // ── Server relay ─────────────────────────────────────────────────────
                    // Only the active client's packets are forwarded to the server.
                    if client_id == self.active_client {
                        if let Err(e) = self.remote_tx.send(packet).await {
                            self.log
                                .error(format!("Ошибка отправки пакета на сервер: {}", e));
                            return;
                        }
                    }
                }

                Event::ClientDisconnected(client_id) => {
                    match client_id {
                        ClientId::Primary => {
                            self.primary_active = false;
                            self.log.client_status("primary", false);
                        }
                        ClientId::Secondary => {
                            self.secondary_active = false;
                            self.log.client_status("secondary", false);
                        }
                    }

                    if !(self.primary_active || self.secondary_active) {
                        self.log.info("Оба клиента отключились, сессия завершена");
                        return;
                    }

                    if self.active_client == client_id {
                        // Active client disconnected — switch control to the other one.
                        self.active_client = client_id.opposite();
                        self.log.info(format!(
                            "Активный клиент отключился, управление передано: {}",
                            match self.active_client {
                                ClientId::Primary => "основной",
                                ClientId::Secondary => "дополнительный",
                            }
                        ));

                        if let Some(event) =
                            self.version.handle_client_disconnect(self.active_client)
                        {
                            match event {
                                crate::protocols::ClientDisconnectEvent::SendToServer(packets) => {
                                    for packet in packets {
                                        self.remote_tx.send(packet).await.unwrap();
                                    }
                                }
                            }
                        }
                    }
                }

                Event::ServerData(packet) => {
                    if let Some(event) = self.version.handle_s2c(&packet, self.both_active()) {
                        match event {}
                    }

                    // Broadcast the raw packet to whichever clients are still active.
                    if self.primary_active {
                        let _ = self.primary_tx.send(packet.clone()).await;
                    }
                    if self.secondary_active {
                        let _ = self.secondary_tx.send(packet).await;
                    }
                }
            }
        }
    }

    /// Returns `true` only when both clients are currently connected.
    fn both_active(&self) -> bool {
        self.primary_active && self.secondary_active
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
                }
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
