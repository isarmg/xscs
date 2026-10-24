use async_trait::async_trait;
use serde::Serialize;
use std::{io, sync::Arc, time::Duration};

/// A finite lifecycle, not a user-extensible stage engine.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum ShutdownPhase {
    Running,
    Quiescing,
    DrainingRequests,
    CancellingOrdinaryWork,
    DrainingCommits,
    ClosingState,
    Stopped,
}

#[derive(Clone, Debug, Serialize)]
pub struct ShutdownReport {
    pub clean: bool,
    pub phase: ShutdownPhase,
    pub unfinished_connections: usize,
    pub unfinished_ordinary_work: usize,
    pub unfinished_commits: usize,
}

/// Business obligations only. Implementations must never own process signals.
#[async_trait]
pub trait LifecycleParticipant: Send + Sync + 'static {
    fn quiesce(&self);
    fn cancel_ordinary_work(&self);
    fn active_tasks(&self) -> (usize, usize);
    /// Wait until request registration is closed and ordinary work has ended.
    async fn drain_requests(&self);
    /// Close commit registration only after every possible producer has ended.
    async fn drain_commits(&self);
    /// Called only after all requests, ordinary work, and commits have drained.
    async fn close_state(&self) -> Result<(), String>;
}

/// Retains business state/locks after an incomplete shutdown. The executable
/// must exit nonzero without releasing this owner while obligations remain.
pub struct ShutdownFailure {
    pub report: ShutdownReport,
    pub(crate) _retained: Option<Arc<dyn LifecycleParticipant>>,
}

impl std::fmt::Debug for ShutdownFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.report.fmt(f)
    }
}
impl std::fmt::Display for ShutdownFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "incomplete shutdown: {:?}", self.report)
    }
}
impl std::error::Error for ShutdownFailure {}

/// Install before binding or constructing synchronous product state.
pub struct ProcessSignals {
    interrupt: tokio::signal::unix::Signal,
    terminate: tokio::signal::unix::Signal,
}

impl ProcessSignals {
    pub fn install() -> io::Result<Self> {
        use tokio::signal::unix::{SignalKind, signal};
        Ok(Self {
            interrupt: signal(SignalKind::interrupt())?,
            terminate: signal(SignalKind::terminate())?,
        })
    }
    pub async fn recv(&mut self) -> io::Result<&'static str> {
        tokio::select! {
            value = self.interrupt.recv() => value.map(|()| "SIGINT"),
            value = self.terminate.recv() => value.map(|()| "SIGTERM"),
        }
        .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "process signal stream closed"))
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ShutdownLimits {
    pub grace: Duration,
    pub forced: Duration,
}
impl Default for ShutdownLimits {
    fn default() -> Self {
        Self {
            grace: Duration::from_secs(30),
            forced: Duration::from_secs(10),
        }
    }
}
