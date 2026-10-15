use std::{
    fmt::Write as _,
    time::{Duration, Instant},
};

use crate::{diag, events::PerClient};

const REPORT_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, Default)]
pub struct Flow {
    pub packets: u64,
    pub bytes: u64,
}

impl Flow {
    pub fn add(&mut self, bytes: usize) {
        self.packets += 1;
        self.bytes += bytes as u64;
    }
}

/// Plain counters owned by the controller task; reported periodically and on drop.
pub struct SessionStats {
    started: Instant,
    last_report: Instant,
    pub from_client: PerClient<Flow>,
    pub to_server: Flow,
    pub from_server: Flow,
    pub to_client: PerClient<Flow>,
    pub to_inactive: Flow,
    pub skipped: PerClient<u64>,
    pub phase_switches: u64,
    pub phase_acks: PerClient<u64>,
    pub caught_up: PerClient<u64>,
    pub held_peak_bytes: PerClient<usize>,
    pub stalled_drops: PerClient<u64>,
    pub failovers: u64,
    pub replayed: Flow,
}

impl SessionStats {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            started: now,
            last_report: now,
            from_client: PerClient::default(),
            to_server: Flow::default(),
            from_server: Flow::default(),
            to_client: PerClient::default(),
            to_inactive: Flow::default(),
            skipped: PerClient::default(),
            phase_switches: 0,
            phase_acks: PerClient::default(),
            caught_up: PerClient::default(),
            held_peak_bytes: PerClient::default(),
            stalled_drops: PerClient::default(),
            failovers: 0,
            replayed: Flow::default(),
        }
    }

    pub fn maybe_report(&mut self) {
        if self.last_report.elapsed() >= REPORT_INTERVAL {
            self.report("periodic");
        }
    }

    pub fn report(&mut self, label: &str) {
        self.last_report = Instant::now();
        diag::info("stats", self.summary(label));
    }

    fn summary(&self, label: &str) -> String {
        let mut out = String::new();
        let _ = write!(
            out,
            "{label} after {:?}: c2s primary={:?} secondary={:?} -> server={:?}; \
             s2c from server={:?} -> primary={:?} secondary={:?}; to_inactive={:?} \
             skipped={:?} phase_switches={} acks={:?} caught_up={:?} held_peak={:?} \
             stalled_drops={:?} failovers={} replayed={:?}",
            self.started.elapsed(),
            self.from_client.primary,
            self.from_client.secondary,
            self.to_server,
            self.from_server,
            self.to_client.primary,
            self.to_client.secondary,
            self.to_inactive,
            self.skipped,
            self.phase_switches,
            self.phase_acks,
            self.caught_up,
            self.held_peak_bytes,
            self.stalled_drops,
            self.failovers,
            self.replayed
        );
        out
    }
}

impl Drop for SessionStats {
    fn drop(&mut self) {
        self.report("final");
    }
}
