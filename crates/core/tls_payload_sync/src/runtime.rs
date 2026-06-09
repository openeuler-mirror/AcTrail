//! Runtime-side sync event writer.

use std::collections::VecDeque;
use std::fmt;
use std::io::{BufWriter, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;

use crate::{SyncError, SyncEvent, SyncResult, write_event_line};

pub struct EventClient {
    queue: Arc<EventQueue>,
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
        thread::Builder::new()
            .name("actrail-tls-sync-event-writer".to_string())
            .spawn(move || event_writer(stream, worker_queue))
            .map_err(|error| SyncError::new(format!("spawn sync event writer: {error}")))?;
        Ok(Self { queue })
    }

    pub fn send(&self, event: SyncEvent) -> SyncResult<()> {
        self.queue.push(event)
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
                closed: false,
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
        if state.closed {
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
        while state.pending.is_empty() && !state.closed {
            state = self.ready.wait(state).ok()?;
        }
        let queued = state.pending.pop_front()?;
        state.pending_bytes = state.pending_bytes.saturating_sub(queued.event_bytes);
        Some(queued)
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
    closed: bool,
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
            queue.close();
            return;
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
    }
}
