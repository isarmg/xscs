//! Bounded HTTP/1 transport with connection limits, deadlines and graceful shutdown.
//! No filesystem, authentication, operation, or product-state semantics live here.

use crate::log::{CommonEvent, LogRecord};
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use std::{
    future::Future,
    io::{self, IoSlice},
    net::{SocketAddr, TcpListener as StdTcpListener},
    pin::Pin,
    sync::Arc,
    task::{Context as TaskContext, Poll},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf, unix::AsyncFd},
    net::TcpStream,
    sync::{OwnedSemaphorePermit, Semaphore},
    time::sleep,
};
use tokio_util::sync::CancellationToken;

const ACCEPT_BACKOFF_INITIAL: Duration = Duration::from_millis(50);
const ACCEPT_BACKOFF_MAX: Duration = Duration::from_secs(1);

pub(crate) struct WriteIdleTimeout<T> {
    inner: T,
    timeout: Duration,
    timer: Option<Pin<Box<tokio::time::Sleep>>>,
    // The permit must follow the socket even when HTTP upgrades transfer it
    // out of the connection future (e.g. a managed WebSocket).
    _permit: Option<OwnedSemaphorePermit>,
    forced: Option<Pin<Box<dyn Future<Output = ()> + Send>>>,
}

impl<T> WriteIdleTimeout<T> {
    pub(crate) fn new(inner: T, timeout: Duration) -> Self {
        debug_assert!(!timeout.is_zero());
        Self {
            inner,
            timeout,
            timer: None,
            _permit: None,
            forced: None,
        }
    }

    fn clear_timer(&mut self) {
        self.timer = None;
    }

    pub(crate) fn with_permit(
        mut self,
        permit: OwnedSemaphorePermit,
        force: CancellationToken,
    ) -> Self {
        self._permit = Some(permit);
        self.forced = Some(Box::pin(force.cancelled_owned()));
        self
    }

    fn check_force(&mut self, context: &mut TaskContext<'_>) -> io::Result<()> {
        if self
            .forced
            .as_mut()
            .is_some_and(|forced| forced.as_mut().poll(context).is_ready())
        {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "HTTP transport forced shutdown",
            ));
        }
        Ok(())
    }

    fn write_is_timed_out(&mut self, context: &mut TaskContext<'_>) -> bool {
        let timer = self
            .timer
            .get_or_insert_with(|| Box::pin(tokio::time::sleep(self.timeout)));
        timer.as_mut().poll(context).is_ready()
    }

    fn timeout_error() -> io::Error {
        io::Error::new(
            io::ErrorKind::TimedOut,
            "HTTP response write made no progress before the idle deadline",
        )
    }
}

impl<T: AsyncRead + Unpin> AsyncRead for WriteIdleTimeout<T> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut TaskContext<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        self.check_force(context)?;
        Pin::new(&mut self.inner).poll_read(context, buffer)
    }
}

impl<T: AsyncWrite + Unpin> AsyncWrite for WriteIdleTimeout<T> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut TaskContext<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.as_mut().get_mut();
        this.check_force(context)?;
        match Pin::new(&mut this.inner).poll_write(context, buffer) {
            Poll::Ready(result) => {
                this.clear_timer();
                Poll::Ready(result)
            }
            Poll::Pending if this.write_is_timed_out(context) => {
                Poll::Ready(Err(Self::timeout_error()))
            }
            Poll::Pending => Poll::Pending,
        }
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        context: &mut TaskContext<'_>,
        buffers: &[IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        let this = self.as_mut().get_mut();
        this.check_force(context)?;
        match Pin::new(&mut this.inner).poll_write_vectored(context, buffers) {
            Poll::Ready(result) => {
                this.clear_timer();
                Poll::Ready(result)
            }
            Poll::Pending if this.write_is_timed_out(context) => {
                Poll::Ready(Err(Self::timeout_error()))
            }
            Poll::Pending => Poll::Pending,
        }
    }

    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }

    fn poll_flush(mut self: Pin<&mut Self>, context: &mut TaskContext<'_>) -> Poll<io::Result<()>> {
        let this = self.as_mut().get_mut();
        this.check_force(context)?;
        match Pin::new(&mut this.inner).poll_flush(context) {
            Poll::Ready(result) => {
                this.clear_timer();
                Poll::Ready(result)
            }
            Poll::Pending if this.write_is_timed_out(context) => {
                Poll::Ready(Err(Self::timeout_error()))
            }
            Poll::Pending => Poll::Pending,
        }
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        context: &mut TaskContext<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.as_mut().get_mut();
        this.check_force(context)?;
        match Pin::new(&mut this.inner).poll_shutdown(context) {
            Poll::Ready(result) => {
                this.clear_timer();
                Poll::Ready(result)
            }
            Poll::Pending if this.write_is_timed_out(context) => {
                Poll::Ready(Err(Self::timeout_error()))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

pub(crate) async fn accept_with_backoff(
    listener: &AsyncFd<StdTcpListener>,
    service: &str,
    listener_addr: &str,
    shutdown: &CancellationToken,
    connection_slots: &Arc<Semaphore>,
    backoff: &mut AcceptBackoff,
) -> Option<(TcpStream, OwnedSemaphorePermit)> {
    loop {
        let readiness = tokio::select! {
            biased;
            _ = shutdown.cancelled() => return None,
            result = listener.readable() => result,
        };
        let mut readiness = match readiness {
            Ok(readiness) => readiness,
            Err(err) => {
                if !wait_after_accept_error(service, listener_addr, shutdown, backoff, &err).await {
                    return None;
                }
                continue;
            }
        };

        // AsyncFd deliberately keeps a successful readiness observation cached
        // so callers can drain an fd. We cannot drain a listener before owning
        // capacity, however: waiting for a permit on that stale cache would let
        // an idle listener starve another bind. Recheck the kernel's level state
        // without accepting; clear only the stale observation.
        match listener_is_readable(listener.get_ref()) {
            Ok(true) => {}
            Ok(false) => {
                readiness.clear_ready();
                continue;
            }
            Err(err) => {
                drop(readiness);
                if !wait_after_accept_error(service, listener_addr, shutdown, backoff, &err).await {
                    return None;
                }
                continue;
            }
        }

        // Wait for a global slot only after this listener has a connection
        // ready. An idle bind therefore cannot reserve capacity from another
        // address, while every socket accepted into userspace already owns the
        // permit that bounds its lifetime.
        let connection_permit = tokio::select! {
            biased;
            _ = shutdown.cancelled() => return None,
            permit = connection_slots.clone().acquire_owned() => {
                permit.expect("the connection semaphore is never closed")
            }
        };
        if shutdown.is_cancelled() {
            return None;
        }

        let accepted = match readiness.try_io(|listener| listener.get_ref().accept()) {
            Ok(result) => result,
            Err(_) => continue,
        };
        drop(readiness);
        match accepted {
            Ok((stream, _)) => {
                if let Err(err) = stream.set_nonblocking(true) {
                    drop(connection_permit);
                    if !wait_after_accept_error(service, listener_addr, shutdown, backoff, &err)
                        .await
                    {
                        return None;
                    }
                    continue;
                }
                let stream = match TcpStream::from_std(stream) {
                    Ok(stream) => stream,
                    Err(err) => {
                        drop(connection_permit);
                        if !wait_after_accept_error(service, listener_addr, shutdown, backoff, &err)
                            .await
                        {
                            return None;
                        }
                        continue;
                    }
                };
                backoff.reset();
                if shutdown.is_cancelled() {
                    return None;
                }
                return Some((stream, connection_permit));
            }
            Err(err) => {
                drop(connection_permit);
                if !wait_after_accept_error(service, listener_addr, shutdown, backoff, &err).await {
                    return None;
                }
            }
        }
    }
}

fn listener_is_readable(listener: &StdTcpListener) -> io::Result<bool> {
    let mut descriptors = [PollFd::new(listener, PollFlags::IN)];
    let no_wait = Timespec::default();
    let ready = poll(&mut descriptors, Some(&no_wait)).map_err(io::Error::from)?;
    Ok(ready > 0 && !descriptors[0].revents().is_empty())
}

async fn wait_after_accept_error(
    service: &str,
    listener_addr: &str,
    shutdown: &CancellationToken,
    backoff: &mut AcceptBackoff,
    err: &io::Error,
) -> bool {
    let retry_delay = backoff.failure_delay();
    log_accept_error(service, listener_addr, err, retry_delay);
    tokio::select! {
        biased;
        _ = shutdown.cancelled() => false,
        _ = sleep(retry_delay) => true,
    }
}

#[derive(Debug)]
pub(crate) struct AcceptBackoff {
    next_delay: Duration,
}

impl Default for AcceptBackoff {
    fn default() -> Self {
        Self {
            next_delay: ACCEPT_BACKOFF_INITIAL,
        }
    }
}

impl AcceptBackoff {
    fn failure_delay(&mut self) -> Duration {
        let delay = self.next_delay;
        self.next_delay = self.next_delay.saturating_mul(2).min(ACCEPT_BACKOFF_MAX);
        delay
    }

    fn reset(&mut self) {
        self.next_delay = ACCEPT_BACKOFF_INITIAL;
    }
}

fn log_accept_error(
    service: &str,
    listener_addr: &str,
    err: &std::io::Error,
    retry_delay: Duration,
) {
    let category = classify_accept_error(err);
    let record = LogRecord::common(service, CommonEvent::HttpAcceptFailed)
        .and_then(|record| record.with_attribute("listener", listener_addr))
        .and_then(|record| record.with_attribute("category", category))
        .and_then(|record| record.with_attribute("io_kind", format!("{:?}", err.kind())))
        .and_then(|record| record.with_attribute("os_error", serde_json::json!(err.raw_os_error())))
        .and_then(|record| record.with_attribute("retry_ms", retry_delay.as_millis() as u64))
        .expect("validated service and bounded transport attributes form a log record");
    emit_record(&record);
}

fn classify_accept_error(err: &std::io::Error) -> &'static str {
    if is_resource_exhaustion(err) {
        "resource"
    } else {
        match err.kind() {
            std::io::ErrorKind::Interrupted => "interrupted",
            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut => "transient",
            std::io::ErrorKind::ConnectionAborted | std::io::ErrorKind::ConnectionReset => {
                "connection"
            }
            std::io::ErrorKind::PermissionDenied => "permission",
            std::io::ErrorKind::AddrNotAvailable | std::io::ErrorKind::NotConnected => "listener",
            _ => "io",
        }
    }
}

fn is_resource_exhaustion(err: &std::io::Error) -> bool {
    if err.kind() == std::io::ErrorKind::OutOfMemory {
        return true;
    }
    // Linux ENOMEM, ENFILE, EMFILE and ENOBUFS.
    matches!(err.raw_os_error(), Some(12 | 23 | 24 | 105))
}

pub(crate) fn log_connection_error(
    service: &str,
    request_seen: bool,
    err: &(dyn std::error::Error + Send + Sync + 'static),
) {
    let hyper_error = err.downcast_ref::<hyper::Error>();
    let io_error = find_io_error(err);
    let io_kind = io_error.map(std::io::Error::kind);
    let io_disconnect = matches!(
        io_kind,
        Some(
            std::io::ErrorKind::ConnectionReset
                | std::io::ErrorKind::ConnectionAborted
                | std::io::ErrorKind::BrokenPipe
                | std::io::ErrorKind::UnexpectedEof
                | std::io::ErrorKind::NotConnected
        )
    );
    let benign_probe_close =
        hyper_error.is_some_and(hyper::Error::is_incomplete_message) || io_disconnect;
    if !request_seen && benign_probe_close {
        return;
    }

    let category = if hyper_error.is_some_and(hyper::Error::is_parse) {
        "protocol"
    } else if hyper_error.is_some_and(hyper::Error::is_timeout)
        || io_kind == Some(std::io::ErrorKind::TimedOut)
    {
        "timeout"
    } else if hyper_error.is_some_and(|err| err.is_user() || err.is_body_write_aborted()) {
        "service"
    } else if hyper_error
        .is_some_and(|err| err.is_incomplete_message() || err.is_canceled() || err.is_closed())
        || io_disconnect
    {
        "disconnect"
    } else if io_error.is_some() {
        "io"
    } else if hyper_error.is_some() {
        "hyper"
    } else {
        "unknown"
    };
    let io_kind = io_kind
        .map(|kind| format!("{kind:?}"))
        .unwrap_or_else(|| "-".to_string());
    let event = if category == "disconnect" {
        CommonEvent::HttpConnectionClosed
    } else {
        CommonEvent::HttpConnectionFailed
    };
    let record = LogRecord::common(service, event)
        .and_then(|record| record.with_attribute("category", category))
        .and_then(|record| record.with_attribute("request_seen", request_seen))
        .and_then(|record| record.with_attribute("io_kind", io_kind))
        .expect("validated service and bounded transport attributes form a log record");
    emit_record(&record);
}

fn emit_record(record: &LogRecord) {
    if record.emit().is_err() {
        // The same structured record remains observable when the persistent
        // sink fails. No raw error chain, request contents or peer address.
        let _ = record.emit_stderr();
    }
}

fn find_io_error<'a>(
    err: &'a (dyn std::error::Error + Send + Sync + 'static),
) -> Option<&'a std::io::Error> {
    let mut current: Option<&'a (dyn std::error::Error + 'static)> = Some(err);
    for _ in 0..16 {
        let Some(error) = current else { break };
        if let Some(io_error) = error.downcast_ref::<std::io::Error>() {
            return Some(io_error);
        }
        current = error.source();
    }
    None
}

pub(crate) fn create_listener(addr: SocketAddr) -> io::Result<AsyncFd<StdTcpListener>> {
    use socket2::{Domain, Protocol, Socket, Type};
    let socket = Socket::new(Domain::for_address(addr), Type::STREAM, Some(Protocol::TCP))?;
    if addr.is_ipv6() {
        socket.set_only_v6(true)?;
    }
    socket.set_reuse_address(true)?;
    socket.bind(&addr.into())?;
    socket.listen(1024 /* Default backlog */)?;
    let std_listener = StdTcpListener::from(socket);
    std_listener.set_nonblocking(true)?;
    let listener = AsyncFd::new(std_listener)?;
    Ok(listener)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    #[tokio::test(start_paused = true)]
    async fn write_idle_timeout_interrupts_a_stalled_peer() {
        let (stream, _peer) = tokio::io::duplex(1);
        let mut stream = WriteIdleTimeout::new(stream, Duration::from_secs(30));
        let error = stream.write_all(b"ab").await.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    }

    #[test]
    fn accept_backoff_is_bounded_and_resets() {
        let mut backoff = AcceptBackoff::default();
        assert_eq!(backoff.failure_delay(), Duration::from_millis(50));
        assert_eq!(backoff.failure_delay(), Duration::from_millis(100));
        for _ in 0..20 {
            assert!(backoff.failure_delay() <= Duration::from_secs(1));
        }
        backoff.reset();
        assert_eq!(backoff.failure_delay(), Duration::from_millis(50));
    }

    #[tokio::test]
    async fn backoff_is_interrupted_by_shutdown() {
        let stop = CancellationToken::new();
        stop.cancel();
        assert!(
            !wait_after_accept_error(
                "example-product",
                "test",
                &stop,
                &mut AcceptBackoff::default(),
                &io::Error::from_raw_os_error(24)
            )
            .await
        );
    }
}
