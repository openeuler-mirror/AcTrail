//! Runtime-side sync event writer.

use std::collections::VecDeque;
use std::fmt;
use std::io::{BufWriter, Write};
use std::os::fd::RawFd;
use std::path::Path;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[path = "runtime/hold.rs"]
mod hold;
#[path = "runtime/transport.rs"]
mod transport;

pub use hold::ResponseHold;
pub(crate) use transport::{DeadlineStream, Endpoint};

use crate::{EventRegistration, FrameCodec, SyncError, SyncEvent, SyncResult};
use tls_payload_core::PayloadDirection;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EventClientConfig {
    pub pending_byte_budget: usize,
    pub max_frame_bytes: usize,
    pub timeout: Duration,
    pub shutdown_timeout: Duration,
    pub batch_bytes: usize,
}

/// Only atomic loads and close(2) are used by the runtime's atfork child handler.
#[derive(Debug)]
pub struct EventTransportFds {
    writer: AtomicI32,
    reader: AtomicI32,
}

impl Default for EventTransportFds {
    fn default() -> Self {
        Self {
            writer: AtomicI32::new(-1),
            reader: AtomicI32::new(-1),
        }
    }
}

impl EventTransportFds {
    pub fn close_after_fork(&self) {
        for slot in [&self.writer, &self.reader] {
            let fd = slot.swap(-1, Ordering::Relaxed);
            if fd >= 0 {
                unsafe { libc::close(fd) };
            }
        }
    }

    fn register(self: &Arc<Self>, fd: RawFd, reader: bool) -> FdRegistration {
        let slot = if reader { &self.reader } else { &self.writer };
        slot.store(fd, Ordering::Release);
        FdRegistration {
            fds: Arc::clone(self),
            reader,
        }
    }
}

pub(super) struct FdRegistration {
    fds: Arc<EventTransportFds>,
    reader: bool,
}

impl Drop for FdRegistration {
    fn drop(&mut self) {
        let slot = if self.reader {
            &self.fds.reader
        } else {
            &self.fds.writer
        };
        slot.store(-1, Ordering::Release);
    }
}

pub struct EventClient {
    queue: Arc<EventQueue>,
    worker: Mutex<Option<JoinHandle<()>>>,
    hold: Option<Arc<ResponseHold>>,
    shutdown_timeout: Duration,
}

impl EventClient {
    /// With `hold`, the client also reads the daemon's releases into it. `config.timeout` bounds
    /// sender progress; closing the client uses the separate configured drain budget.
    pub fn connect(
        path: &Path,
        config: EventClientConfig,
        registration: EventRegistration,
        hold: Option<Arc<ResponseHold>>,
        fds: Arc<EventTransportFds>,
    ) -> SyncResult<Self> {
        Self::start(
            Endpoint(path.to_path_buf()),
            config,
            registration,
            hold,
            fds,
        )
    }

    fn start(
        endpoint: Endpoint,
        config: EventClientConfig,
        registration: EventRegistration,
        hold: Option<Arc<ResponseHold>>,
        fds: Arc<EventTransportFds>,
    ) -> SyncResult<Self> {
        if config.pending_byte_budget == 0 || config.batch_bytes == 0 {
            return Err(SyncError::new(
                "sync event pending byte budget and batch bytes must be positive",
            ));
        }
        if config.max_frame_bytes < FrameCodec::HEADER_LEN {
            return Err(SyncError::new(
                "TLS frame limit must include the frame header",
            ));
        }
        for timeout in [config.timeout, config.shutdown_timeout] {
            if timeout.is_zero() || Instant::now().checked_add(timeout).is_none() {
                return Err(SyncError::new(
                    "sync event timeouts must be positive representable durations",
                ));
            }
        }
        let queue = Arc::new(EventQueue::new(config.pending_byte_budget));
        let worker_queue = Arc::clone(&queue);
        let worker_hold = hold.clone();
        let worker = thread::Builder::new()
            .name("actrail-tls-sync-event-writer".to_string())
            .spawn(move || {
                EventWriter {
                    endpoint,
                    queue: worker_queue,
                    config,
                    registration,
                    hold: worker_hold,
                    fds,
                }
                .run()
            })
            .map_err(|error| SyncError::new(format!("spawn sync event writer: {error}")))?;
        Ok(Self {
            queue,
            worker: Mutex::new(Some(worker)),
            hold,
            shutdown_timeout: config.shutdown_timeout,
        })
    }

    pub fn send(&self, event: SyncEvent) -> SyncResult<()> {
        let outbound = self.hold.as_ref().and(outbound_sequence(&event));
        self.queue.push(event)?;
        if let (Some(hold), Some((stream_key, sequence))) = (&self.hold, outbound) {
            hold.sent(stream_key, sequence);
        }
        Ok(())
    }

    pub fn close(&self) -> SyncResult<()> {
        let deadline = Instant::now() + self.shutdown_timeout;
        self.queue.close(deadline);
        let result = self.queue.flush_until(deadline);
        if result.is_err() {
            self.queue.fail();
        }
        if let Some(worker) = self
            .worker
            .lock()
            .map_err(|_| SyncError::new("sync event worker mutex poisoned"))?
            .take()
        {
            // A stopped receiver must never extend the caller's exit deadline.
            if worker.is_finished() {
                worker
                    .join()
                    .map_err(|_| SyncError::new("sync event writer panicked"))?;
            }
        }
        result
    }
}

impl fmt::Debug for EventClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EventClient")
            .finish_non_exhaustive()
    }
}

struct EventQueue {
    state: Mutex<EventQueueState>,
    ready: Condvar,
    pending_byte_budget: usize,
}

impl EventQueue {
    fn new(pending_byte_budget: usize) -> Self {
        Self {
            state: Mutex::new(EventQueueState {
                pending: VecDeque::new(),
                pending_bytes: 0,
                in_flight: 0,
                close_deadline: None,
                failed: false,
                dropped: 0,
            }),
            ready: Condvar::new(),
            pending_byte_budget,
        }
    }

    fn push(&self, event: SyncEvent) -> SyncResult<()> {
        let event_bytes = queued_event_bytes(&event);
        let mut state = self
            .state
            .lock()
            .map_err(|_| SyncError::new("sync event queue mutex poisoned"))?;
        if state.close_deadline.is_some() || state.failed {
            state.dropped = state.dropped.saturating_add(1);
            return Err(SyncError::new("sync event writer is closed"));
        }
        let next_pending_bytes = state
            .pending_bytes
            .checked_add(event_bytes)
            .ok_or_else(|| SyncError::new("sync event queue byte count overflow"))?;
        if state.pending_bytes != 0 && next_pending_bytes > self.pending_byte_budget {
            state.dropped = state.dropped.saturating_add(1);
            return Err(SyncError::new("sync event queue is full"));
        }
        state.pending_bytes = next_pending_bytes;
        state.pending.push_back(QueuedEvent { event, event_bytes });
        self.ready.notify_one();
        Ok(())
    }

    fn pop_batch(&self, byte_limit: usize) -> Vec<QueuedEvent> {
        let Ok(mut state) = self.state.lock() else {
            return Vec::new();
        };
        loop {
            if state.failed {
                return Vec::new();
            }
            if !state.pending.is_empty() {
                let mut batch = Vec::new();
                let mut batch_bytes = 0usize;
                while let Some(next) = state.pending.front() {
                    let next_bytes = batch_bytes.saturating_add(next.event_bytes);
                    if !batch.is_empty() && next_bytes > byte_limit {
                        break;
                    }
                    let queued = state.pending.pop_front().expect("front exists");
                    state.pending_bytes = state.pending_bytes.saturating_sub(queued.event_bytes);
                    batch_bytes = next_bytes;
                    batch.push(queued);
                }
                state.in_flight = state.in_flight.saturating_add(batch.len());
                return batch;
            }
            if state.close_deadline.is_some() {
                return Vec::new();
            }
            let Ok(next_state) = self.ready.wait(state) else {
                return Vec::new();
            };
            state = next_state;
        }
    }

    fn finish_batch(&self, count: usize) {
        if let Ok(mut state) = self.state.lock() {
            state.in_flight = state.in_flight.saturating_sub(count);
            self.ready.notify_all();
        }
    }

    fn fail(&self) {
        if let Ok(mut state) = self.state.lock() {
            if !state.failed {
                state.dropped = state
                    .dropped
                    .saturating_add(state.pending.len() as u64)
                    .saturating_add(state.in_flight as u64);
            }
            state.failed = true;
            state.pending.clear();
            state.pending_bytes = 0;
            state.in_flight = 0;
            self.ready.notify_all();
        }
    }

    fn flush_until(&self, deadline: Instant) -> SyncResult<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| SyncError::new("sync event queue mutex poisoned"))?;
        while (!state.pending.is_empty() || state.in_flight != 0) && !state.failed {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(SyncError::new("sync event shutdown deadline exceeded"));
            }
            (state, _) = self
                .ready
                .wait_timeout(state, remaining)
                .map_err(|_| SyncError::new("sync event queue mutex poisoned"))?;
        }
        if state.failed {
            Err(SyncError::new("sync event writer is closed"))
        } else {
            Ok(())
        }
    }

    fn close(&self, deadline: Instant) {
        if let Ok(mut state) = self.state.lock() {
            state.close_deadline = Some(
                state
                    .close_deadline
                    .map_or(deadline, |old| old.min(deadline)),
            );
            self.ready.notify_all();
        }
    }

    fn deadline(&self, progress_deadline: Instant) -> std::io::Result<Instant> {
        let state = self
            .state
            .lock()
            .map_err(|_| std::io::Error::other("sync event queue poisoned"))?;
        if state.failed {
            return Err(std::io::Error::other("sync event transport disabled"));
        }
        Ok(state
            .close_deadline
            .map_or(progress_deadline, |close| close.min(progress_deadline)))
    }

    fn dropped(&self) -> u64 {
        self.state.lock().map(|state| state.dropped).unwrap_or(0)
    }
}

struct EventQueueState {
    pending: VecDeque<QueuedEvent>,
    pending_bytes: usize,
    in_flight: usize,
    close_deadline: Option<Instant>,
    failed: bool,
    dropped: u64,
}

struct QueuedEvent {
    event: SyncEvent,
    event_bytes: usize,
}

struct EventWriter {
    endpoint: Endpoint,
    queue: Arc<EventQueue>,
    config: EventClientConfig,
    registration: EventRegistration,
    hold: Option<Arc<ResponseHold>>,
    fds: Arc<EventTransportFds>,
}

impl EventWriter {
    fn run(self) {
        let queue = Arc::clone(&self.queue);
        let hold = self.hold.clone();
        let result = self.write();
        if result.is_err() {
            queue.fail();
        }
        // Without the transport no release can come.
        if let Some(hold) = hold {
            hold.end();
        }
        let dropped = queue.dropped();
        if result.is_err() || dropped != 0 {
            // Best effort and once per transport; no producer-side logging.
            Endpoint::diagnostic(&format!(
                "actrail tls-sync: transport {}; unconfirmed_events={dropped}; {}\n",
                if result.is_err() {
                    "disabled"
                } else {
                    "closed"
                },
                result
                    .err()
                    .map(|error| error.to_string())
                    .unwrap_or_default()
            ));
        }
    }

    fn write(self) -> SyncResult<()> {
        let deadline = self.queue.deadline(Instant::now() + self.config.timeout)?;
        let stream = self.endpoint.connect(deadline)?;
        use std::os::fd::AsRawFd;
        let registration = self.fds.register(stream.as_raw_fd(), false);
        if let Some(hold) = &self.hold {
            let reader = stream.try_clone()?;
            let reader_registration = self.fds.register(reader.as_raw_fd(), true);
            let hold = Arc::clone(hold);
            let max_frame_bytes = self.config.max_frame_bytes;
            thread::Builder::new()
                .name("actrail-tls-sync-release-reader".to_string())
                .spawn(move || hold.read_releases(reader, max_frame_bytes, reader_registration))?;
        }
        let mut stream = DeadlineStream::for_events(
            stream,
            self.config.timeout,
            Arc::clone(&self.queue),
            registration,
            self.config.batch_bytes,
        );
        stream.set_deadline(self.queue.deadline(Instant::now() + self.config.timeout)?);
        let mut writer = BufWriter::with_capacity(self.config.batch_bytes, stream);
        FrameCodec::write_registration(
            &mut writer,
            &self.registration,
            self.hold.is_some(),
            self.config.max_frame_bytes,
        )?;
        writer.flush()?;
        loop {
            let batch = self.queue.pop_batch(self.config.batch_bytes);
            if batch.is_empty() {
                return Ok(());
            }
            // Start a new idle window; writes also observe a later close deadline.
            let deadline = self.queue.deadline(Instant::now() + self.config.timeout)?;
            writer.get_mut().set_deadline(deadline);
            for queued in &batch {
                FrameCodec::write_event(&mut writer, &queued.event, self.config.max_frame_bytes)?;
            }
            writer.flush()?;
            self.queue.finish_batch(batch.len());
        }
    }
}

/// The connection and sequence of an event that carries bytes the process wrote.
fn outbound_sequence(event: &SyncEvent) -> Option<(u64, u64)> {
    match event {
        SyncEvent::Payload(event) if event.direction == PayloadDirection::Outbound => {
            Some((event.stream_key, event.sequence))
        }
        SyncEvent::Summary(event) if event.direction == PayloadDirection::Outbound => {
            Some((event.stream_key, event.sequence))
        }
        _ => None,
    }
}

fn queued_event_bytes(event: &SyncEvent) -> usize {
    match event {
        SyncEvent::Payload(event) => event
            .bytes
            .len()
            .saturating_add(event.provider.len())
            .saturating_add(event.symbol.len()),
        SyncEvent::Decision(event) => event
            .reason
            .len()
            .saturating_add(event.provider.len())
            .saturating_add(event.symbol.len())
            .saturating_add(event.action.len()),
        SyncEvent::Summary(event) => event
            .bytes
            .len()
            .saturating_add(event.provider.len())
            .saturating_add(event.symbol.len())
            .saturating_add(event.reason.len())
            .saturating_add(event.protocol_hint.len()),
    }
}
