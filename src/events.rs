use std::ops::{Index, IndexMut};

use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Success,
    Warn,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientId {
    Primary,
    Secondary,
}

impl ClientId {
    pub fn opposite(self) -> ClientId {
        match self {
            ClientId::Primary => ClientId::Secondary,
            ClientId::Secondary => ClientId::Primary,
        }
    }
}

/// One value per client, indexed by [`ClientId`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PerClient<T> {
    pub primary: T,
    pub secondary: T,
}

impl<T> PerClient<T> {
    pub fn new(primary: T, secondary: T) -> Self {
        Self { primary, secondary }
    }

    pub fn splat(value: T) -> Self
    where
        T: Clone,
    {
        Self::new(value.clone(), value)
    }

    pub fn iter(&self) -> std::array::IntoIter<&T, 2> {
        [&self.primary, &self.secondary].into_iter()
    }

    pub fn iter_mut(&mut self) -> std::array::IntoIter<&mut T, 2> {
        [&mut self.primary, &mut self.secondary].into_iter()
    }
}

impl<'a, T> IntoIterator for &'a PerClient<T> {
    type Item = &'a T;
    type IntoIter = std::array::IntoIter<&'a T, 2>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a, T> IntoIterator for &'a mut PerClient<T> {
    type Item = &'a mut T;
    type IntoIter = std::array::IntoIter<&'a mut T, 2>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

impl<T> Index<ClientId> for PerClient<T> {
    type Output = T;

    fn index(&self, client: ClientId) -> &T {
        match client {
            ClientId::Primary => &self.primary,
            ClientId::Secondary => &self.secondary,
        }
    }
}

impl<T> IndexMut<ClientId> for PerClient<T> {
    fn index_mut(&mut self, client: ClientId) -> &mut T {
        match client {
            ClientId::Primary => &mut self.primary,
            ClientId::Secondary => &mut self.secondary,
        }
    }
}

#[derive(Debug, Clone)]
pub enum AppEvent {
    Log { level: LogLevel, message: String },
    SessionStarted,
    SessionEnded,
    ClientStatus { which: ClientId, online: bool },
    NickName(String),
    ServerAddr(String),
    UpdateProgress { downloaded: u64, total: u64 },
    UpdateVerifying { waited_secs: u64 },
}

/// Cloneable handle the backend uses to push events to the UI.
#[derive(Clone)]
pub struct EventSink(UnboundedSender<AppEvent>);

impl EventSink {
    pub fn channel() -> (Self, UnboundedReceiver<AppEvent>) {
        let (tx, rx) = unbounded();
        (Self(tx), rx)
    }

    pub fn emit(&self, event: AppEvent) {
        let _ = self.0.unbounded_send(event);
    }

    pub fn client_status(&self, which: ClientId, online: bool) {
        self.emit(AppEvent::ClientStatus { which, online });
    }
}
