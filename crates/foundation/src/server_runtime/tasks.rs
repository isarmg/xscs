//! Request admission and non-aborting obligation tracking. No business state.
use std::{
    future::Future,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::{
    sync::{OwnedRwLockReadGuard, RwLock},
    task::JoinHandle,
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

#[derive(Clone, Debug, Default)]
pub struct TrackedTasks {
    tracker: TaskTracker,
    closed: Arc<Mutex<bool>>,
}

#[derive(Clone, Copy, Debug, thiserror::Error)]
#[error("task registration is closed")]
pub struct WorkClosed;

impl TrackedTasks {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn try_spawn<F>(&self, future: F) -> Result<JoinHandle<F::Output>, WorkClosed>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        let closed = self.closed.lock().expect("task registration lock poisoned");
        if *closed {
            return Err(WorkClosed);
        }
        // Registration and closing are serialized. The future owns its real
        // I/O resources; dropping a JoinHandle never cancels this obligation.
        Ok(self.tracker.spawn(future))
    }
    /// For producers already protected by an admitted request or tracked work.
    /// Closing before those producers finish is a caller invariant violation.
    pub fn spawn<F>(&self, future: F) -> JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.try_spawn(future)
            .expect("producer attempted registration after drain")
    }
    pub fn close(&self) {
        *self.closed.lock().expect("task registration lock poisoned") = true;
        self.tracker.close();
    }
    pub fn is_closed(&self) -> bool {
        *self.closed.lock().expect("task registration lock poisoned")
    }
    pub fn len(&self) -> usize {
        self.tracker.len()
    }
    pub fn is_empty(&self) -> bool {
        self.tracker.is_empty()
    }
    pub async fn wait(&self) {
        self.tracker.wait().await;
    }
}

#[derive(Clone)]
pub struct WorkScope {
    pub running: Arc<AtomicBool>,
    pub work_tasks: TrackedTasks,
    pub commit_tasks: TrackedTasks,
    pub shutdown: CancellationToken,
    pub force_shutdown: CancellationToken,
    request_gate: Arc<RwLock<()>>,
}
impl Default for WorkScope {
    fn default() -> Self {
        Self {
            running: Arc::new(AtomicBool::new(true)),
            work_tasks: TrackedTasks::new(),
            commit_tasks: TrackedTasks::new(),
            shutdown: CancellationToken::new(),
            force_shutdown: CancellationToken::new(),
            request_gate: Arc::new(RwLock::new(())),
        }
    }
}
impl WorkScope {
    pub fn new() -> Self {
        Self::default()
    }
    pub async fn enter_request(&self) -> Option<OwnedRwLockReadGuard<()>> {
        let guard = tokio::select! {
            biased;
            _ = self.shutdown.cancelled() => return None,
            guard = self.request_gate.clone().read_owned() => guard,
        };
        (!self.shutdown.is_cancelled()).then_some(guard)
    }
    pub fn quiesce(&self) {
        self.shutdown.cancel();
    }
    pub fn cancel_ordinary_work(&self) {
        self.quiesce();
        self.running.store(false, Ordering::SeqCst);
        self.force_shutdown.cancel();
    }
    pub async fn drain_requests(&self) {
        self.quiesce();
        let _requests = self.request_gate.write().await;
        self.work_tasks.close();
        self.work_tasks.wait().await;
    }
    pub async fn drain_commits(&self) {
        self.commit_tasks.close();
        self.commit_tasks.wait().await;
        self.running.store(false, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::poll;
    use std::task::Poll;

    #[tokio::test]
    async fn admitted_request_can_register_commit_before_drain_closes_registration() {
        let scope = WorkScope::new();
        let request = scope.enter_request().await.unwrap();
        let mut drain = Box::pin(scope.drain_requests());
        assert!(matches!(poll!(&mut drain), Poll::Pending));
        assert!(scope.enter_request().await.is_none());
        let (finish, pending) = tokio::sync::oneshot::channel::<()>();
        scope.commit_tasks.spawn(async {
            pending.await.unwrap();
        });
        drop(request);
        drain.await;
        let mut commits = Box::pin(scope.drain_commits());
        assert!(matches!(poll!(&mut commits), Poll::Pending));
        assert!(scope.commit_tasks.try_spawn(async {}).is_err());
        assert!(scope.work_tasks.try_spawn(async {}).is_err());
        finish.send(()).unwrap();
        commits.await;
    }

    #[tokio::test]
    async fn cancelling_waiter_does_not_release_running_blocking_work_resources() {
        let work = TrackedTasks::new();
        let slots = Arc::new(tokio::sync::Semaphore::new(1));
        let permit = slots.clone().acquire_owned().await.unwrap();
        let (started, entered) = tokio::sync::oneshot::channel();
        let (release, blocked) = std::sync::mpsc::channel();
        let waiter = work.spawn(async move {
            tokio::task::spawn_blocking(move || {
                let _permit = permit;
                started.send(()).unwrap();
                blocked.recv().unwrap();
            })
            .await
            .unwrap();
        });
        entered.await.unwrap();
        drop(waiter);
        work.close();
        assert_eq!(slots.available_permits(), 0);
        assert_eq!(work.len(), 1);
        release.send(()).unwrap();
        work.wait().await;
        assert_eq!(slots.available_permits(), 1);
    }
}
