use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{RecvTimeoutError, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::{ExportDeliveryDrop, ExportError, ExportPublishResult};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BestEffortDeliveryConfig {
    pub component_name: &'static str,
    pub worker_thread_name: &'static str,
    pub queue_capacity: u32,
    /// Maximum time `finish()` may wait for the worker. `None` drains fully.
    pub shutdown_timeout: Option<Duration>,
}

pub trait BestEffortSink<T>: Send + 'static {
    /// Supplies the shared shutdown signal used by the delivery worker.
    /// Sinks with retry loops should consult it between blocking operations.
    fn bind_shutdown(&mut self, _shutdown: BestEffortShutdown) {}

    /// Returns the number of records made durable by this call.
    fn deliver(&mut self, message: T) -> Result<u64, String>;

    /// Time until this sink next needs worker attention without a new message.
    /// `None` keeps the worker blocked until input arrives or the sender closes.
    fn idle_timeout(&self) -> Option<Duration> {
        None
    }

    /// Called when `idle_timeout` elapses and returns records made durable.
    fn on_idle(&mut self) -> Result<u64, String> {
        Ok(u64::default())
    }

    /// Flushes buffered records and returns the number made durable.
    fn finish(&mut self) -> Result<u64, String> {
        Ok(u64::default())
    }
}

pub struct BestEffortDelivery<T: Send + 'static> {
    state: Mutex<BestEffortDeliveryState<T>>,
    accepted_records: AtomicU64,
    durable_records: Arc<AtomicU64>,
    error: Arc<Mutex<Option<String>>>,
    component_name: &'static str,
    queue_capacity: u32,
    shutdown_timeout: Option<Duration>,
    shutdown: BestEffortShutdown,
}

#[derive(Clone, Default)]
pub struct BestEffortShutdown {
    deadline: Arc<Mutex<Option<Instant>>>,
}

impl BestEffortShutdown {
    pub fn deadline(&self) -> Option<Instant> {
        self.deadline.lock().ok().and_then(|deadline| *deadline)
    }

    pub fn expired(&self) -> bool {
        match self.deadline.lock() {
            Ok(deadline) => deadline.is_some_and(|deadline| Instant::now() >= deadline),
            // A poisoned shutdown signal cannot safely coordinate more I/O.
            Err(_) => true,
        }
    }

    fn set_deadline(&self, deadline: Option<Instant>) -> Result<(), String> {
        let mut slot = self
            .deadline
            .lock()
            .map_err(|error| format!("shutdown deadline lock poisoned: {error}"))?;
        *slot = deadline;
        Ok(())
    }
}

struct BestEffortDeliveryState<T> {
    sender: Option<SyncSender<T>>,
    worker: Option<JoinHandle<()>>,
    finished: bool,
}

#[derive(Debug)]
pub struct BestEffortDeliveryFinish {
    dropped_records: u64,
    error: Option<ExportError>,
}

impl BestEffortDeliveryFinish {
    pub const fn dropped_records(&self) -> u64 {
        self.dropped_records
    }

    pub const fn error(&self) -> Option<&ExportError> {
        self.error.as_ref()
    }

    const fn empty() -> Self {
        Self {
            dropped_records: 0,
            error: None,
        }
    }
}

impl<T: Send + 'static> BestEffortDelivery<T> {
    pub fn spawn<S>(
        config: BestEffortDeliveryConfig,
        sink: S,
    ) -> Result<BestEffortDelivery<T>, ExportError>
    where
        S: BestEffortSink<T>,
    {
        if config.queue_capacity == u32::default() {
            return Err(ExportError::new(
                config.component_name,
                "queue capacity must be positive",
            ));
        }
        let queue_capacity = usize::try_from(config.queue_capacity).map_err(|error| {
            ExportError::new(
                config.component_name,
                format!("queue capacity overflow: {error}"),
            )
        })?;
        let (sender, receiver) = sync_channel(queue_capacity);
        let error = Arc::new(Mutex::new(None));
        let thread_error = Arc::clone(&error);
        let durable_records = Arc::new(AtomicU64::new(0));
        let thread_durable_records = Arc::clone(&durable_records);
        let shutdown = BestEffortShutdown::default();
        let thread_shutdown = shutdown.clone();
        let worker = thread::Builder::new()
            .name(config.worker_thread_name.to_string())
            .spawn(move || {
                let mut sink = sink;
                sink.bind_shutdown(thread_shutdown.clone());
                loop {
                    if thread_shutdown.expired() {
                        break;
                    }
                    match sink.idle_timeout() {
                        Some(timeout) => match receiver.recv_timeout(timeout) {
                            Ok(message) => match sink.deliver(message) {
                                Ok(delivered) => {
                                    thread_durable_records.fetch_add(delivered, Ordering::Relaxed);
                                }
                                Err(error) => {
                                    store_delivery_error(&thread_error, error);
                                    return;
                                }
                            },
                            Err(RecvTimeoutError::Timeout) => match sink.on_idle() {
                                Ok(delivered) => {
                                    thread_durable_records.fetch_add(delivered, Ordering::Relaxed);
                                }
                                Err(error) => {
                                    store_delivery_error(&thread_error, error);
                                    return;
                                }
                            },
                            Err(RecvTimeoutError::Disconnected) => break,
                        },
                        None => match receiver.recv() {
                            Ok(message) => match sink.deliver(message) {
                                Ok(delivered) => {
                                    thread_durable_records.fetch_add(delivered, Ordering::Relaxed);
                                }
                                Err(error) => {
                                    store_delivery_error(&thread_error, error);
                                    return;
                                }
                            },
                            Err(_) => break,
                        },
                    }
                }
                if !thread_shutdown.expired() {
                    match sink.finish() {
                        Ok(delivered) => {
                            thread_durable_records.fetch_add(delivered, Ordering::Relaxed);
                        }
                        Err(error) => {
                            store_delivery_error(&thread_error, error);
                        }
                    }
                }
            })
            .map_err(|error| {
                ExportError::new(
                    config.component_name,
                    format!("spawn delivery worker failed: {error}"),
                )
            })?;

        Ok(Self {
            state: Mutex::new(BestEffortDeliveryState {
                sender: Some(sender),
                worker: Some(worker),
                finished: false,
            }),
            accepted_records: AtomicU64::new(0),
            durable_records,
            error,
            component_name: config.component_name,
            queue_capacity: config.queue_capacity,
            shutdown_timeout: config.shutdown_timeout,
            shutdown,
        })
    }

    pub fn check_health(&self) -> Result<(), ExportError> {
        let error = self.error.lock().map_err(|error| {
            self.delivery_error(format!("delivery error lock poisoned: {error}"))
        })?;
        match error.as_ref() {
            Some(message) => Err(self.delivery_error(message.clone())),
            None => Ok(()),
        }
    }

    pub fn publish(&self, message: T) -> Result<ExportPublishResult, ExportError> {
        self.check_health()?;
        let state = self.state.lock().map_err(|error| {
            self.delivery_error(format!("delivery state lock poisoned: {error}"))
        })?;
        let Some(sender) = &state.sender else {
            return Err(self.delivery_error("delivery sender is closed"));
        };
        match sender.try_send(message) {
            Ok(()) => {
                self.accepted_records.fetch_add(1, Ordering::Relaxed);
                Ok(ExportPublishResult::delivered())
            }
            Err(TrySendError::Full(_)) => Ok(ExportPublishResult::dropped(
                ExportDeliveryDrop::queue_full(1, self.queue_capacity),
            )),
            Err(TrySendError::Disconnected(_)) => {
                self.check_health()?;
                Err(self.delivery_error("delivery worker disconnected"))
            }
        }
    }

    pub fn finish(&self) -> BestEffortDeliveryFinish {
        let (worker, mut errors, deadline) = {
            let (mut state, state_error) = match self.state.lock() {
                Ok(state) => (state, None),
                Err(error) => {
                    let message = format!("delivery state lock poisoned: {error}");
                    (error.into_inner(), Some(message))
                }
            };
            if state.finished {
                return BestEffortDeliveryFinish::empty();
            }
            state.finished = true;
            let deadline = self
                .shutdown_timeout
                .and_then(|timeout| Instant::now().checked_add(timeout));
            let deadline_error = self.shutdown.set_deadline(deadline).err();
            state.sender.take();
            (
                state.worker.take(),
                state_error
                    .into_iter()
                    .chain(deadline_error)
                    .collect::<Vec<_>>(),
                deadline,
            )
        };

        if let Some(worker) = worker {
            match deadline {
                Some(deadline) => {
                    while !worker.is_finished() && Instant::now() < deadline {
                        let remaining = deadline.saturating_duration_since(Instant::now());
                        thread::sleep(remaining.min(Duration::from_millis(1)));
                    }
                    if worker.is_finished() {
                        if worker.join().is_err() {
                            errors.push("delivery worker panicked".to_string());
                        }
                    } else {
                        errors.push(
                            "shutdown deadline exceeded; delivery worker detached".to_string(),
                        );
                    }
                }
                None => {
                    if worker.join().is_err() {
                        errors.push("delivery worker panicked".to_string());
                    }
                }
            }
        }
        match self.error.lock() {
            Ok(error) => errors.extend(error.iter().cloned()),
            Err(error) => errors.push(format!("delivery error lock poisoned: {error}")),
        }

        let accepted_records = self.accepted_records.load(Ordering::Relaxed);
        let durable_records = self.durable_records.load(Ordering::Relaxed);
        let dropped_records = accepted_records.saturating_sub(durable_records);
        if durable_records > accepted_records {
            errors.push(format!(
                "delivery sink acknowledged {durable_records} record(s), but only \
                 {accepted_records} were accepted"
            ));
        } else if dropped_records > 0 && errors.is_empty() {
            errors.push(format!(
                "delivery worker finished with {dropped_records} unacknowledged record(s)"
            ));
        }
        let error = (!errors.is_empty()).then(|| self.delivery_error(errors.join("; ")));
        BestEffortDeliveryFinish {
            dropped_records,
            error,
        }
    }

    fn delivery_error(&self, message: impl Into<String>) -> ExportError {
        ExportError::new(self.component_name, message).with_queue_capacity(self.queue_capacity)
    }
}

impl<T: Send + 'static> Drop for BestEffortDelivery<T> {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

fn store_delivery_error(error: &Arc<Mutex<Option<String>>>, message: String) {
    if let Ok(mut slot) = error.lock() {
        *slot = Some(message);
    }
}
