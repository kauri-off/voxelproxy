use std::{collections::VecDeque, sync::Arc};

use mc_protocol::packet::RawPacket;
use tokio::{
    io::{AsyncWriteExt, BufReader, BufWriter},
    net::tcp::{OwnedReadHalf, OwnedWriteHalf},
    sync::{
        OwnedSemaphorePermit, Semaphore, TryAcquireError,
        mpsc::{self, Receiver, Sender, UnboundedReceiver, UnboundedSender},
    },
};

use crate::{
    events::{ClientId, EventSink},
    logger::Logger,
    protocols::{ClientBoundEvent, ServerBoundEvent, Version, VersionProtocol},
};

const IO_BUFFER_SIZE: usize = 64 * 1024;
/// Bytes a peer may have queued but not yet written before it is considered stalled.
const PEER_QUEUE_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug)]
pub enum ControllerEvent {
    ClientData(ClientId, RawPacket),
    ClientDisconnected(ClientId),
    ServerData(RawPacket),
    ServerDisconnected,
}

/// A packet queued for a writer task; the permit returns its bytes to the budget once written.
pub type Outgoing = (Arc<RawPacket>, OwnedSemaphorePermit);

enum TrySendError {
    Full,
    Closed,
}

/// Unbounded packet queue to a writer task, limited by the total number of queued bytes.
pub struct PeerLink {
    tx: UnboundedSender<Outgoing>,
    budget: Arc<Semaphore>,
}

impl PeerLink {
    pub fn new() -> (Self, UnboundedReceiver<Outgoing>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let budget = Arc::new(Semaphore::new(PEER_QUEUE_BYTES));
        (Self { tx, budget }, rx)
    }

    fn cost(packet: &RawPacket) -> u32 {
        packet.data.len().clamp(1, PEER_QUEUE_BYTES) as u32
    }

    /// Waits while the peer is over budget. Returns `false` if its writer is gone.
    async fn send(&self, packet: Arc<RawPacket>) -> bool {
        let Ok(permit) = self
            .budget
            .clone()
            .acquire_many_owned(Self::cost(&packet))
            .await
        else {
            return false;
        };
        self.tx.send((packet, permit)).is_ok()
    }

    fn try_send(&self, packet: Arc<RawPacket>) -> Result<(), TrySendError> {
        let permit = match self
            .budget
            .clone()
            .try_acquire_many_owned(Self::cost(&packet))
        {
            Ok(p) => p,
            Err(TryAcquireError::NoPermits) => return Err(TrySendError::Full),
            Err(TryAcquireError::Closed) => return Err(TrySendError::Closed),
        };
        self.tx
            .send((packet, permit))
            .map_err(|_| TrySendError::Closed)
    }
}

/// Packets for a client that has not acked a phase switch yet, like a vanilla server would wait.
#[derive(Default)]
struct HeldPackets {
    queue: VecDeque<(Arc<RawPacket>, bool)>,
    bytes: usize,
}

pub struct Controller {
    active_client: ClientId,
    held: [Option<HeldPackets>; 2],
    /// Queues to the clients, indexed by `ClientId`; `None` once a client went offline.
    peers: [Option<PeerLink>; 2],
    /// Channel for sending packets to the upstream server.
    remote_tx: Sender<RawPacket>,
    /// Receives events (client data, disconnections, server data) from I/O tasks.
    event_rx: Receiver<ControllerEvent>,
    version: Version,
    events: EventSink,
}

impl Controller {
    pub fn new(
        active_client: ClientId,
        primary: PeerLink,
        secondary: PeerLink,
        remote_tx: Sender<RawPacket>,
        event_rx: Receiver<ControllerEvent>,
        version: Version,
        events: EventSink,
    ) -> Self {
        Self {
            active_client,
            held: [None, None],
            peers: [Some(primary), Some(secondary)],
            remote_tx,
            event_rx,
            version,
            events,
        }
    }

    pub async fn run(mut self) {
        let log = Logger::new(&self.events);
        while let Some(event) = self.event_rx.recv().await {
            match event {
                ControllerEvent::ClientData(client_id, packet) => {
                    let event = self.version.handle_c2s(
                        &packet,
                        client_id,
                        client_id == self.active_client,
                        self.both_online(),
                    );

                    let mut skip = false;

                    if let Some(event) = event {
                        match event {
                            ServerBoundEvent::SendToInactive(raw_packet) => {
                                if self.both_online() {
                                    self.relay(
                                        self.active_client.opposite(),
                                        Arc::new(raw_packet),
                                        false,
                                    )
                                    .await;
                                }
                            }
                            ServerBoundEvent::SkipRelay => skip = true,
                            ServerBoundEvent::PhaseAcked => self.release(client_id).await,
                        }
                    }

                    if skip {
                        continue;
                    }

                    if client_id == self.active_client
                        && let Err(e) = self.remote_tx.send(packet).await
                    {
                        log.error(format!("Ошибка отправки пакета на сервер: {}", e));
                        return;
                    }
                }

                ControllerEvent::ClientDisconnected(client_id) => {
                    self.mark_offline(client_id);

                    if self.peers.iter().all(Option::is_none) {
                        return;
                    }

                    if self.active_client == client_id {
                        self.active_client = client_id.opposite();
                        log.info(format!(
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
                                        if self.remote_tx.send(packet).await.is_err() {
                                            log.error("Соединение с сервером потеряно");
                                            return;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                ControllerEvent::ServerData(packet) => {
                    let phase_switch = matches!(
                        self.version.handle_s2c(&packet, self.both_online()),
                        Some(ClientBoundEvent::PhaseSwitch)
                    );

                    let packet = Arc::new(packet);
                    let inactive = self.active_client.opposite();
                    self.relay(self.active_client, packet.clone(), phase_switch)
                        .await;
                    self.relay(inactive, packet, phase_switch).await;
                }

                // Ending here also tears down half-open clients that never sent a FIN.
                ControllerEvent::ServerDisconnected => return,
            }
        }
    }

    /// Delivers a packet unless the client is still switching phase; the vanilla client
    /// crashes on packets of a phase it has not switched its outbound protocol to yet.
    async fn relay(&mut self, client_id: ClientId, packet: Arc<RawPacket>, phase_switch: bool) {
        if let Some(held) = &mut self.held[client_id.index()] {
            held.bytes += packet.data.len();
            held.queue.push_back((packet, phase_switch));
            if held.bytes > PEER_QUEUE_BYTES {
                self.drop_stalled(client_id);
            }
            return;
        }
        self.deliver(client_id, packet).await;
        if phase_switch && self.peers[client_id.index()].is_some() {
            self.held[client_id.index()] = Some(HeldPackets::default());
        }
    }

    /// Sends what was held for the client, up to and including the next phase switch.
    async fn release(&mut self, client_id: ClientId) {
        let Some(mut held) = self.held[client_id.index()].take() else {
            return;
        };
        while let Some((packet, phase_switch)) = held.queue.pop_front() {
            held.bytes -= packet.data.len();
            self.deliver(client_id, packet).await;
            if phase_switch {
                self.held[client_id.index()] = Some(held);
                return;
            }
        }
    }

    /// The active client gets backpressure; an inactive one that falls too far behind
    /// is dropped instead, so it can never stall the session.
    async fn deliver(&mut self, client_id: ClientId, packet: Arc<RawPacket>) {
        let Some(link) = &self.peers[client_id.index()] else {
            return;
        };
        if client_id == self.active_client {
            link.send(packet).await;
            return;
        }
        if let Err(TrySendError::Full) = link.try_send(packet) {
            self.drop_stalled(client_id);
        }
    }

    fn drop_stalled(&mut self, client_id: ClientId) {
        Logger::new(&self.events).warn("Клиент не успевает принимать данные и был отключён");
        self.mark_offline(client_id);
    }

    /// Dropping the queue ends its writer task, which closes the socket's write half.
    fn mark_offline(&mut self, client_id: ClientId) {
        self.held[client_id.index()] = None;
        self.peers[client_id.index()] = None;
        self.events.client_status(client_id, false);
    }

    fn both_online(&self) -> bool {
        self.peers.iter().all(Option::is_some)
    }
}

trait PacketQueue {
    type Item;

    async fn recv(&mut self) -> Option<Self::Item>;
    fn try_recv(&mut self) -> Option<Self::Item>;
    fn packet(item: &Self::Item) -> &RawPacket;
}

impl PacketQueue for UnboundedReceiver<Outgoing> {
    type Item = Outgoing;

    async fn recv(&mut self) -> Option<Outgoing> {
        UnboundedReceiver::recv(self).await
    }

    fn try_recv(&mut self) -> Option<Outgoing> {
        UnboundedReceiver::try_recv(self).ok()
    }

    fn packet((packet, _permit): &Outgoing) -> &RawPacket {
        packet
    }
}

impl PacketQueue for Receiver<RawPacket> {
    type Item = RawPacket;

    async fn recv(&mut self) -> Option<RawPacket> {
        Receiver::recv(self).await
    }

    fn try_recv(&mut self) -> Option<RawPacket> {
        Receiver::try_recv(self).ok()
    }

    fn packet(packet: &RawPacket) -> &RawPacket {
        packet
    }
}

/// Writes queued packets through one buffer, flushing only once the queue is drained.
async fn write_loop<Q: PacketQueue>(
    write_half: OwnedWriteHalf,
    mut queue: Q,
) -> anyhow::Result<()> {
    let mut writer = BufWriter::with_capacity(IO_BUFFER_SIZE, write_half);
    while let Some(item) = queue.recv().await {
        Q::packet(&item).write_async(&mut writer).await?;
        while let Some(item) = queue.try_recv() {
            Q::packet(&item).write_async(&mut writer).await?;
        }
        writer.flush().await?;
    }
    Ok(())
}

/// Forwards packets as events until the stream fails, then reports `eof_event`.
async fn read_loop(
    read_half: OwnedReadHalf,
    event_tx: Sender<ControllerEvent>,
    on_packet: impl Fn(RawPacket) -> ControllerEvent,
    eof_event: ControllerEvent,
) {
    let mut reader = BufReader::with_capacity(IO_BUFFER_SIZE, read_half);
    loop {
        match RawPacket::read_async(&mut reader).await {
            Ok(packet) => {
                if event_tx.send(on_packet(packet)).await.is_err() {
                    return;
                }
            }
            Err(_) => {
                event_tx.send(eof_event).await.ok();
                return;
            }
        }
    }
}

pub async fn run_client(
    read_half: OwnedReadHalf,
    write_half: OwnedWriteHalf,
    client_id: ClientId,
    event_tx: Sender<ControllerEvent>,
    packet_rx: UnboundedReceiver<Outgoing>,
) {
    let _ = tokio::join!(
        read_loop(
            read_half,
            event_tx,
            |packet| ControllerEvent::ClientData(client_id, packet),
            ControllerEvent::ClientDisconnected(client_id),
        ),
        write_loop(write_half, packet_rx)
    );
}

pub async fn run_server(
    read_half: OwnedReadHalf,
    write_half: OwnedWriteHalf,
    event_tx: Sender<ControllerEvent>,
    packet_rx: Receiver<RawPacket>,
) {
    let _ = tokio::join!(
        read_loop(
            read_half,
            event_tx,
            ControllerEvent::ServerData,
            ControllerEvent::ServerDisconnected,
        ),
        write_loop(write_half, packet_rx)
    );
}
