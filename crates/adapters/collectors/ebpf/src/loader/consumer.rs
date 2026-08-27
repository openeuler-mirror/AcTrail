//! Dedicated kernel-event consumer thread.
//!
//! The kernel ring/perf buffer is a multi-producer, single-consumer transport:
//! exactly one thread may drain it. The daemon's event loop must not pause
//! draining while it runs the expensive ingest pipeline (decoding, semantic
//! projection, SQLite persistence), so this module owns the transport buffer
//! on a dedicated thread that greedily drains it into bounded raw batches and
//! hands them to the daemon through a bounded channel.
//!
//! The kernel buffer is drained continuously instead of only at the edges of
//! a drain cycle, which removes the starvation window that used to end in
//! `reserve_fail` data loss. An eventfd wakes the daemon whenever a batch is
//! queued; when the queue is full the consumer applies backpressure by
//! blocking, letting the kernel ring buffer absorb the burst.

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, sync_channel};
use std::thread::JoinHandle;

use libbpf_rs::MapHandle;

use super::LoaderError;
use super::object::EventBuffer;

/// Maximum raw bytes carried by one queued batch.
const MAX_BATCH_BYTES: usize = 2 * 1024 * 1024;
/// Maximum records carried by one queued batch.
const MAX_BATCH_RECORDS: usize = 4096;
/// Number of batches the bounded queue holds before the consumer applies
/// backpressure (blocks), letting the kernel ring buffer absorb more.
const QUEUE_BATCH_CAPACITY: usize = 32;
/// Poll timeout in milliseconds; doubles as a watchdog so a missed kernel
/// wakeup is still drained and perf-lost counters stay current.
const POLL_TIMEOUT_MS: i32 = 250;

pub(crate) enum EventConsumerMessage {
    RawBatch { raw: Vec<Vec<u8>>, perf_lost: u64 },
    Failure { stage: String, message: String },
}

pub(crate) struct EventConsumer {
    messages: Option<Receiver<EventConsumerMessage>>,
    wake_fd: OwnedFd,
    shutdown_fd: OwnedFd,
    thread: Option<JoinHandle<()>>,
}

impl EventConsumer {
    pub(crate) fn spawn(events_map: &MapHandle, buffer_bytes: u32) -> Result<Self, LoaderError> {
        // The batch Vec lives on a stable heap location (Box) so the raw
        // pointer captured by the transport callback stays valid while the
        // Box is moved into the consumer thread.
        let mut batch: Box<Vec<Vec<u8>>> = Box::new(Vec::new());
        let batch_ptr = &mut *batch as *mut Vec<Vec<u8>>;
        let event_buffer = EventBuffer::build_with_sink(events_map, buffer_bytes, move |raw| {
            // SAFETY: consume() invokes this callback synchronously on the
            // consumer thread while that thread owns `batch`; the Box heap
            // allocation is stable across the move into the thread, and no
            // other thread touches the Vec.
            unsafe { (*batch_ptr).push(raw.to_vec()) };
        })?;

        let (messages_tx, messages_rx) = sync_channel(QUEUE_BATCH_CAPACITY);
        let wake_fd = new_eventfd("wake")?;
        let shutdown_fd = new_eventfd("shutdown")?;
        let event_poll_fd = event_buffer.epoll_fd();
        if event_poll_fd < 0 {
            return Err(LoaderError::new(
                "event_consumer",
                format!("event buffer returned invalid epoll fd {event_poll_fd}"),
            ));
        }
        let wake_raw = wake_fd.as_raw_fd();
        let shutdown_raw = shutdown_fd.as_raw_fd();
        let thread = std::thread::Builder::new()
            .name("actrail-ebpf-event-consumer".to_string())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run_event_consumer(
                        &event_buffer,
                        batch,
                        &messages_tx,
                        wake_raw,
                        shutdown_raw,
                        event_poll_fd,
                    );
                }));
                if let Err(panic) = result {
                    let _ = messages_tx.try_send(EventConsumerMessage::Failure {
                        stage: "consumer_panic".to_string(),
                        message: panic_message(&panic),
                    });
                }
            })
            .map_err(|error| LoaderError::new("event_consumer_spawn", error.to_string()))?;
        Ok(Self {
            messages: Some(messages_rx),
            wake_fd,
            shutdown_fd,
            thread: Some(thread),
        })
    }

    pub(crate) fn try_recv(&self) -> Result<EventConsumerMessage, TryRecvError> {
        self.messages
            .as_ref()
            .map(Receiver::try_recv)
            .unwrap_or(Err(TryRecvError::Disconnected))
    }

    pub(crate) fn wake_fd(&self) -> RawFd {
        self.wake_fd.as_raw_fd()
    }

    /// Reset the level-triggered wakeup counter before draining the queue.
    ///
    /// A consumer write that races with this read leaves the counter
    /// non-zero, so the daemon wakes again for data queued mid-drain.
    pub(crate) fn clear_wakeup(&self) {
        drain_eventfd(self.wake_fd.as_raw_fd());
    }
}

impl Drop for EventConsumer {
    fn drop(&mut self) {
        if let Some(thread) = self.thread.take() {
            // Drop the receiver first so a consumer blocked on a full queue
            // unblocks, then wake it from ppoll.
            drop(self.messages.take());
            signal_eventfd(self.shutdown_fd.as_raw_fd());
            let _ = thread.join();
        }
    }
}

fn run_event_consumer(
    event_buffer: &EventBuffer,
    mut batch: Box<Vec<Vec<u8>>>,
    messages_tx: &SyncSender<EventConsumerMessage>,
    wake_fd: RawFd,
    shutdown_fd: RawFd,
    event_poll_fd: RawFd,
) {
    let mut poll_fds = [
        libc::pollfd {
            fd: event_poll_fd,
            events: libc::POLLIN,
            revents: 0,
        },
        libc::pollfd {
            fd: shutdown_fd,
            events: libc::POLLIN,
            revents: 0,
        },
    ];
    let timeout = libc::timespec {
        tv_sec: 0,
        tv_nsec: i64::from(POLL_TIMEOUT_MS) * 1_000_000,
    };
    let mut last_reported_perf_lost = 0u64;
    loop {
        poll_fds[0].revents = 0;
        poll_fds[1].revents = 0;
        let ready = unsafe {
            libc::ppoll(
                poll_fds.as_mut_ptr(),
                poll_fds.len() as libc::nfds_t,
                &timeout,
                std::ptr::null(),
            )
        };
        if ready < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            let _ = messages_tx.try_send(EventConsumerMessage::Failure {
                stage: "consumer_poll".to_string(),
                message: error.to_string(),
            });
            return;
        }
        if poll_fds[1].revents & (libc::POLLIN | libc::POLLERR | libc::POLLHUP) != 0 {
            return;
        }
        let consume_error = event_buffer.consume().err();
        let raw = std::mem::take(&mut *batch);
        let perf_lost = event_buffer.lost_count();
        let perf_lost_changed = perf_lost != last_reported_perf_lost;
        last_reported_perf_lost = perf_lost;
        if !raw.is_empty() || perf_lost_changed {
            if !send_raw_batches(messages_tx, raw, perf_lost, Some(wake_fd)) {
                return;
            }
        }
        if let Some(error) = consume_error {
            let _ = messages_tx.send(EventConsumerMessage::Failure {
                stage: error.stage,
                message: error.message,
            });
            signal_eventfd(wake_fd);
            return;
        }
    }
}

/// Split drained records into bounded batches and queue them.
///
/// Returns `false` when the daemon side is gone (receiver disconnected), in
/// which case the consumer should exit.
fn send_raw_batches(
    messages_tx: &SyncSender<EventConsumerMessage>,
    raw: Vec<Vec<u8>>,
    perf_lost: u64,
    wake_fd: Option<RawFd>,
) -> bool {
    let mut chunk: Vec<Vec<u8>> = Vec::new();
    let mut chunk_bytes = 0usize;
    let raw_was_empty = raw.is_empty();
    for record in raw {
        if !chunk.is_empty()
            && (chunk.len() >= MAX_BATCH_RECORDS
                || chunk_bytes.saturating_add(record.len()) > MAX_BATCH_BYTES)
        {
            if messages_tx
                .send(EventConsumerMessage::RawBatch {
                    raw: std::mem::take(&mut chunk),
                    perf_lost,
                })
                .is_err()
            {
                return false;
            }
            if let Some(wake_fd) = wake_fd {
                signal_eventfd(wake_fd);
            }
            chunk_bytes = 0;
        }
        chunk_bytes = chunk_bytes.saturating_add(record.len());
        chunk.push(record);
    }
    // An empty final batch is a loss-only message: no raw records but the
    // perf-lost total changed, so the daemon still learns about drops.
    if !chunk.is_empty() || raw_was_empty {
        if messages_tx
            .send(EventConsumerMessage::RawBatch {
                raw: chunk,
                perf_lost,
            })
            .is_err()
        {
            return false;
        }
        if let Some(wake_fd) = wake_fd {
            signal_eventfd(wake_fd);
        }
    }
    true
}

fn new_eventfd(label: &str) -> Result<OwnedFd, LoaderError> {
    let fd = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
    if fd < 0 {
        return Err(LoaderError::new(
            "event_consumer",
            format!("{label}: eventfd failed: {}", io::Error::last_os_error()),
        ));
    }
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

fn signal_eventfd(fd: RawFd) {
    let value = 1u64.to_ne_bytes();
    unsafe {
        libc::write(
            fd,
            value.as_ptr() as *const libc::c_void,
            std::mem::size_of::<u64>(),
        );
    }
}

fn drain_eventfd(fd: RawFd) {
    let mut value = [0u8; 8];
    unsafe {
        libc::read(
            fd,
            value.as_mut_ptr() as *mut libc::c_void,
            std::mem::size_of::<u64>(),
        );
    }
}

fn panic_message(panic: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = panic.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = panic.downcast_ref::<String>() {
        message.clone()
    } else {
        "unknown consumer panic".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect_batches(raw: Vec<Vec<u8>>, perf_lost: u64) -> Vec<Vec<Vec<u8>>> {
        // Capacity large enough that send_raw_batches never blocks while
        // the test drains afterwards.
        let (tx, rx) = sync_channel::<EventConsumerMessage>(1024);
        assert!(send_raw_batches(&tx, raw, perf_lost, None));
        drop(tx);
        let mut batches = Vec::new();
        while let Ok(EventConsumerMessage::RawBatch { raw, .. }) = rx.recv() {
            batches.push(raw);
        }
        batches
    }

    #[test]
    fn loss_only_cycle_emits_one_empty_batch() {
        let batches = collect_batches(Vec::new(), 7);
        assert_eq!(batches.len(), 1);
        assert!(batches[0].is_empty());
    }

    #[test]
    fn small_batches_are_not_split() {
        let batches = collect_batches(vec![vec![1u8], vec![2u8]], 0);
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].len(), 2);
    }

    #[test]
    fn batches_split_by_record_count() {
        let raw = (0..MAX_BATCH_RECORDS + 5)
            .map(|i| vec![i as u8; 8])
            .collect::<Vec<_>>();
        let batches = collect_batches(raw, 0);
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].len(), MAX_BATCH_RECORDS);
        assert_eq!(batches[1].len(), 5);
    }

    #[test]
    fn batches_split_by_bytes() {
        let raw = vec![
            vec![0u8; MAX_BATCH_BYTES * 3 / 4],
            vec![1u8; MAX_BATCH_BYTES * 3 / 4],
            vec![2u8; 16],
        ];
        let batches = collect_batches(raw, 0);
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].len(), 1);
        assert_eq!(batches[1].len(), 2);
    }

    #[test]
    fn single_oversized_record_is_not_split() {
        let raw = vec![vec![0u8; MAX_BATCH_BYTES + 1]];
        let batches = collect_batches(raw, 0);
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].len(), 1);
        assert_eq!(batches[0][0].len(), MAX_BATCH_BYTES + 1);
    }
}
