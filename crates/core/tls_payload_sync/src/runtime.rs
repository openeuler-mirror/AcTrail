//! Runtime-side sync event writer.

use std::collections::VecDeque;
use std::fmt;
use std::io::{BufWriter, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};

use crate::{SyncError, SyncEvent, SyncResult, write_event_line};

pub struct EventClient {
    queue: Arc<EventQueue>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl EventClient {
    pub fn connect(path: &Path, pending_byte_budget: usize) -> SyncResult<Self> {
        if pending_byte_budget == 0 {
            return Err(SyncError::new(
                "sync event pending byte budget must be positive",
            ));
        }
        let stream = UnixStream::connect(path)?;
        let queue = Arc::new(EventQueue::new(pending_byte_budget));
        let worker_queue = Arc::clone(&queue);
        let worker = thread::Builder::new()
            .name("actrail-tls-sync-event-writer".to_string())
            .spawn(move || event_writer(stream, worker_queue))
            .map_err(|error| SyncError::new(format!("spawn sync event writer: {error}")))?;
        Ok(Self {
            queue,
            worker: Mutex::new(Some(worker)),
        })
    }

    pub fn send(&self, event: SyncEvent) -> SyncResult<()> {
        self.queue.push(event)
    }

    pub fn close_and_join(&self) -> SyncResult<()> {
        self.queue.close();
        let Some(worker) = self
            .worker
            .lock()
            .map_err(|_| SyncError::new("sync event worker mutex poisoned"))?
            .take()
        else {
            return self.queue.flush();
        };
        worker
            .join()
            .map_err(|_| SyncError::new("sync event writer panicked"))?;
        self.queue.flush()
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
                closed: false,
                failed: false,
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
        if state.closed || state.failed {
            return Err(SyncError::new("sync event writer is closed"));
        }
        let next_pending_bytes = state
            .pending_bytes
            .checked_add(event_bytes)
            .ok_or_else(|| SyncError::new("sync event queue byte count overflow"))?;
        if state.pending_bytes != 0 && next_pending_bytes > self.pending_byte_budget {
            return Err(SyncError::new("sync event queue is full"));
        }
        state.pending_bytes = next_pending_bytes;
        state.pending.push_back(QueuedEvent { event, event_bytes });
        self.ready.notify_one();
        Ok(())
    }

    fn pop(&self) -> Option<QueuedEvent> {
        let mut state = self.state.lock().ok()?;
        loop {
            if let Some(queued) = state.pending.pop_front() {
                state.pending_bytes = state.pending_bytes.saturating_sub(queued.event_bytes);
                state.in_flight = state.in_flight.saturating_add(1);
                return Some(queued);
            }
            if state.closed || state.failed {
                return None;
            }
            state = self.ready.wait(state).ok()?;
        }
    }

    fn finish_one(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.in_flight = state.in_flight.saturating_sub(1);
            self.ready.notify_all();
        }
    }

    fn fail(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.failed = true;
            state.closed = true;
            self.ready.notify_all();
        }
    }

    fn flush(&self) -> SyncResult<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| SyncError::new("sync event queue mutex poisoned"))?;
        while (!state.pending.is_empty() || state.in_flight != 0) && !state.failed {
            state = self
                .ready
                .wait(state)
                .map_err(|_| SyncError::new("sync event queue mutex poisoned"))?;
        }
        if state.failed {
            Err(SyncError::new("sync event writer is closed"))
        } else {
            Ok(())
        }
    }

    fn close(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.closed = true;
            self.ready.notify_all();
        }
    }
}

struct EventQueueState {
    pending: VecDeque<QueuedEvent>,
    pending_bytes: usize,
    in_flight: usize,
    closed: bool,
    failed: bool,
}

struct QueuedEvent {
    event: SyncEvent,
    event_bytes: usize,
}

fn event_writer(stream: UnixStream, queue: Arc<EventQueue>) {
    let mut writer = BufWriter::new(stream);
    while let Some(queued) = queue.pop() {
        if write_event_line(&mut writer, &queued.event)
            .and_then(|_| {
                writer
                    .flush()
                    .map_err(|error| SyncError::new(error.to_string()))
            })
            .is_err()
        {
            queue.fail();
            return;
        }
        queue.finish_one();
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
    }
}
