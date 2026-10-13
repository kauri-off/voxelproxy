use std::sync::Arc;

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
    events::{EventSink, WhichClient},
    logger::Logger,
    protocols::{ServerBoundEvent, Version, VersionProtocol},
};

const IO_BUFFER_SIZE: usize = 64 * 1024;
/// Bytes a peer may have queued but not yet written before it is considered stalled.
const PEER_QUEUE_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientId {
    Primary,
    Secondary,
}

impl ClientId {
    pub fn opposite(&self) -> ClientId {
        match self {
            ClientId::Primary => ClientId::Secondary,
            ClientId::Secondary => ClientId::Primary,
        }
    }

    pub fn index(self) -> usize {
        match self {
            ClientId::Primary => 0,
            ClientId::Secondary => 1,
        }
    }

    fn which(self) -> WhichClient {
        match self {
            ClientId::Primary => WhichClient::Primary,
            ClientId::Secondary => WhichClient::Secondary,
        }
    }
}

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

pub struct Controller {
    active_client: ClientId,
    /// Queue to the Primary client; `None` once it was dropped for stalling.
    primary: Option<PeerLink>,
    /// Queue to the Secondary client; `None` once it was dropped for stalling.
    secondary: Option<PeerLink>,
    /// Channel for sending packets to the upstream server.
    remote_tx: Sender<RawPacket>,
    /// Receives events (client data, disconnections, server data) from I/O tasks.
    event_rx: Receiver<ControllerEvent>,
    primary_active: bool,
    secondary_active: bool,
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
            primary: Some(primary),
            secondary: Some(secondary),
            remote_tx,
            event_rx,
            primary_active: true,
            secondary_active: true,
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
                        self.active_client,
                        self.both_active(),
                    );

                    let mut skip = false;

                    if let Some(event) = event {
                        match event {
                            ServerBoundEvent::SendToInactive(raw_packet) => {
                                if self.both_active() {
                                    self.deliver(
                                        self.active_client.opposite(),
                                        Arc::new(raw_packet),
                                    )
                                    .await;
                                }
                            }
                            ServerBoundEvent::SkipRelay => skip = true,
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

                    if !(self.primary_active || self.secondary_active) {
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
                    if let Some(event) =
                        self.version
                            .handle_s2c(&packet, self.active_client, self.both_active())
                    {
                        match event {}
                    }

                    let packet = Arc::new(packet);
                    let inactive = self.active_client.opposite();
                    self.deliver(self.active_client, packet.clone()).await;
                    self.deliver(inactive, packet).await;
                }

                // Ending here also tears down half-open clients that never sent a FIN.
                ControllerEvent::ServerDisconnected => return,
            }
        }
    }

    /// The active client gets backpressure; an inactive one that falls too far behind
    /// is dropped instead, so it can never stall the session.
    async fn deliver(&mut self, client_id: ClientId, packet: Arc<RawPacket>) {
        if !self.is_online(client_id) {
            return;
        }
        let Some(link) = self.link(client_id) else {
            return;
        };
        if client_id == self.active_client {
            link.send(packet).await;
            return;
        }
        if let Err(TrySendError::Full) = link.try_send(packet) {
            Logger::new(&self.events)
                .warn("Неактивный клиент не успевает принимать данные и был отключён");
            // Dropping the queue ends its writer task, which closes the socket's write half.
            *self.link_mut(client_id) = None;
            self.mark_offline(client_id);
        }
    }

    fn mark_offline(&mut self, client_id: ClientId) {
        match client_id {
            ClientId::Primary => self.primary_active = false,
            ClientId::Secondary => self.secondary_active = false,
        }
        self.events.client_status(client_id.which(), false);
    }

    fn is_online(&self, client_id: ClientId) -> bool {
        match client_id {
            ClientId::Primary => self.primary_active,
            ClientId::Secondary => self.secondary_active,
        }
    }

    fn link(&self, client_id: ClientId) -> Option<&PeerLink> {
        match client_id {
            ClientId::Primary => self.primary.as_ref(),
            ClientId::Secondary => self.secondary.as_ref(),
        }
    }

    fn link_mut(&mut self, client_id: ClientId) -> &mut Option<PeerLink> {
        match client_id {
            ClientId::Primary => &mut self.primary,
            ClientId::Secondary => &mut self.secondary,
        }
    }

    fn both_active(&self) -> bool {
        self.primary_active && self.secondary_active
    }
}

/// Writes queued packets through one buffer, flushing only once the queue is drained.
async fn write_loop(
    write_half: OwnedWriteHalf,
    mut packet_rx: UnboundedReceiver<Outgoing>,
) -> anyhow::Result<()> {
    let mut writer = BufWriter::with_capacity(IO_BUFFER_SIZE, write_half);
    while let Some((packet, _permit)) = packet_rx.recv().await {
        packet.write_async(&mut writer).await?;
        while let Ok((packet, _permit)) = packet_rx.try_recv() {
            packet.write_async(&mut writer).await?;
        }
        writer.flush().await?;
    }
    Ok(())
}

pub async fn run_client(
    read_half: OwnedReadHalf,
    write_half: OwnedWriteHalf,
    client_id: ClientId,
    event_tx: Sender<ControllerEvent>,
    packet_rx: UnboundedReceiver<Outgoing>,
) {
    let mut client_read = BufReader::with_capacity(IO_BUFFER_SIZE, read_half);
    let _ = tokio::join!(
        async move {
            loop {
                match RawPacket::read_async(&mut client_read).await {
                    Ok(packet) => {
                        if event_tx
                            .send(ControllerEvent::ClientData(client_id, packet))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(_) => {
                        event_tx
                            .send(ControllerEvent::ClientDisconnected(client_id))
                            .await
                            .ok();
                        break;
                    }
                }
            }
        },
        write_loop(write_half, packet_rx)
    );
}

pub async fn run_server(
    read_half: OwnedReadHalf,
    write_half: OwnedWriteHalf,
    event_tx: Sender<ControllerEvent>,
    mut packet_rx: Receiver<RawPacket>,
) {
    let mut server_read = BufReader::with_capacity(IO_BUFFER_SIZE, read_half);
    let _ = tokio::join!(
        async move {
            loop {
                match RawPacket::read_async(&mut server_read).await {
                    Ok(packet) => {
                        if event_tx
                            .send(ControllerEvent::ServerData(packet))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(_) => {
                        event_tx
                            .send(ControllerEvent::ServerDisconnected)
                            .await
                            .ok();
                        break;
                    }
                }
            }
        },
        async move {
            let mut writer = BufWriter::with_capacity(IO_BUFFER_SIZE, write_half);
            while let Some(packet) = packet_rx.recv().await {
                packet.write_async(&mut writer).await?;
                while let Ok(packet) = packet_rx.try_recv() {
                    packet.write_async(&mut writer).await?;
                }
                writer.flush().await?;
            }
            anyhow::Ok(())
        }
    );
}
