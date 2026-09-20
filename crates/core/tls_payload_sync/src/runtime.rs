//! Runtime-side sync event writer.

use std::collections::VecDeque;
use std::fmt;
use std::io::{BufWriter, Write};
use std::os::fd::{FromRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[path = "runtime/transport.rs"]
mod transport;

pub(crate) use transport::{DeadlineStream, Endpoint};

use crate::{FrameCodec, SyncError, SyncEvent, SyncResult};

pub struct EventClient {
    queue: Arc<EventQueue>,
    worker: Mutex<Option<JoinHandle<()>>>,
    timeout: Duration,
}

impl EventClient {
    pub fn connect(
        path: &Path,
        pending_byte_budget: usize,
        write_buffer_bytes: usize,
        max_frame_bytes: usize,
        timeout: Duration,
    ) -> SyncResult<Self> {
        Self::start(
            Endpoint::Path(path.to_path_buf()),
            pending_byte_budget,
            write_buffer_bytes,
            max_frame_bytes,
            timeout,
        )
    }

    pub fn connect_inherited_fd(
        fd: RawFd,
        pending_byte_budget: usize,
        write_buffer_bytes: usize,
        max_frame_bytes: usize,
        timeout: Duration,
    ) -> SyncResult<Self> {
        if fd < 0 {
            return Err(SyncError::new(format!(
                "sync event fd must be non-negative: {fd}"
            )));
        }
        set_exec_inheritable(fd)?;
        let stream = unsafe { UnixStream::from_raw_fd(fd) };
        Self::start(
            Endpoint::Inherited(stream),
            pending_byte_budget,
            write_buffer_bytes,
            max_frame_bytes,
            timeout,
        )
    }

    fn start(
        endpoint: Endpoint,
        pending_byte_budget: usize,
        write_buffer_bytes: usize,
        max_frame_bytes: usize,
        timeout: Duration,
    ) -> SyncResult<Self> {
        if pending_byte_budget == 0 {
            return Err(SyncError::new(
                "sync event pending byte budget must be positive",
            ));
        }
        if write_buffer_bytes == 0 {
            return Err(SyncError::new(
                "sync event write buffer bytes must be positive",
            ));
        }
        if max_frame_bytes < FrameCodec::HEADER_LEN {
            return Err(SyncError::new(
                "TLS frame limit must include the frame header",
            ));
        }
        if timeout.is_zero() || Instant::now().checked_add(timeout).is_none() {
            return Err(SyncError::new(
                "sync event timeout must be a positive representable duration",
            ));
        }
        let queue = Arc::new(EventQueue::new(pending_byte_budget));
        let worker_queue = Arc::clone(&queue);
        let worker = thread::Builder::new()
            .name("actrail-tls-sync-event-writer".to_string())
            .spawn(move || {
                EventWriter {
                    endpoint,
                    queue: worker_queue,
                    write_buffer_bytes,
                    max_frame_bytes,
                    timeout,
                }
                .run()
            })
            .map_err(|error| SyncError::new(format!("spawn sync event writer: {error}")))?;
        Ok(Self {
            queue,
            worker: Mutex::new(Some(worker)),
            timeout,
        })
    }

    pub fn send(&self, event: SyncEvent) -> SyncResult<()> {
        self.queue.push(event)
    }

    pub fn close(&self) -> SyncResult<()> {
        let deadline = Instant::now() + self.timeout;
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

fn set_exec_inheritable(fd: RawFd) -> SyncResult<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 {
        return Err(SyncError::new(format!(
            "read sync event fd flags: {}",
            std::io::Error::last_os_error()
        )));
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) } < 0 {
        return Err(SyncError::new(format!(
            "mark sync event fd inheritable across exec: {}",
            std::io::Error::last_os_error()
        )));
    }
    Ok(())
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

    fn deadline(&self, batch_deadline: Instant) -> std::io::Result<Instant> {
        let state = self
            .state
            .lock()
            .map_err(|_| std::io::Error::other("sync event queue poisoned"))?;
        if state.failed {
            return Err(std::io::Error::other("sync event transport disabled"));
        }
        Ok(state
            .close_deadline
            .map_or(batch_deadline, |close| close.min(batch_deadline)))
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
    write_buffer_bytes: usize,
    max_frame_bytes: usize,
    timeout: Duration,
}

impl EventWriter {
    fn run(self) {
        let queue = Arc::clone(&self.queue);
        let result = self.write();
        if result.is_err() {
            queue.fail();
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
        let deadline = self.queue.deadline(Instant::now() + self.timeout)?;
        let stream = self.endpoint.connect(deadline)?;
        let stream = DeadlineStream::new(stream, self.write_buffer_bytes);
        let mut writer = BufWriter::with_capacity(self.write_buffer_bytes, stream);
        loop {
            let batch = self.queue.pop_batch(self.write_buffer_bytes);
            if batch.is_empty() {
                return Ok(());
            }
            // A later close gets a later deadline; queue state is needed once per batch.
            let deadline = self.queue.deadline(Instant::now() + self.timeout)?;
            writer.get_mut().set_deadline(deadline);
            for queued in &batch {
                FrameCodec::write_event(&mut writer, &queued.event, self.max_frame_bytes)?;
            }
            writer.flush()?;
            self.queue.finish_batch(batch.len());
        }
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
