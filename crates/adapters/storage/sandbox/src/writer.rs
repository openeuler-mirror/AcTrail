use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OpenFlags, TransactionBehavior, params};
use sandbox_evidence_store::{
    NoInterestEvidenceBatch, SandboxEvidenceAdmission, SandboxEvidenceShutdownError,
    SandboxEvidenceWritePort,
};

use crate::codec::ObservationCodec;
use crate::config::SandboxEvidenceSqliteConfig;
use crate::schema::SandboxSchema;
use crate::status::StoreStatus;

pub(super) struct SandboxEvidenceWriter {
    client: Arc<WriterClient>,
    thread: Option<JoinHandle<()>>,
}

impl SandboxEvidenceWriter {
    pub(super) fn start(
        config: SandboxEvidenceSqliteConfig,
        status: Arc<StoreStatus>,
    ) -> Result<Self, String> {
        let queue_capacity = usize::try_from(config.writer_queue_capacity)
            .map_err(|error| format!("sandbox evidence writer queue overflow: {error}"))?;
        let (sender, receiver) = mpsc::sync_channel(queue_capacity);
        let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
        let worker_status = Arc::clone(&status);
        let batch_max_observations = config.batch_max_observations;
        let thread = thread::Builder::new()
            .name("actrail-sandbox-evidence".to_string())
            .stack_size(config.writer_thread_stack_bytes)
            .spawn(move || WriterOwner::new(config, receiver, worker_status).run(ready_sender))
            .map_err(|error| format!("spawn sandbox evidence writer: {error}"))?;
        match ready_receiver.recv() {
            Ok(Ok(())) => Ok(Self {
                client: Arc::new(WriterClient {
                    sender,
                    status,
                    batch_max_observations,
                }),
                thread: Some(thread),
            }),
            Ok(Err(error)) => {
                if thread.join().is_err() {
                    return Err(format!(
                        "{error}; sandbox evidence writer panicked during failed startup"
                    ));
                }
                Err(error)
            }
            Err(error) => {
                let join_message = if thread.join().is_err() {
                    "; writer panicked"
                } else {
                    ""
                };
                Err(format!(
                    "sandbox evidence writer readiness channel closed: {error}{join_message}"
                ))
            }
        }
    }

    pub(super) fn port(&self) -> Arc<dyn SandboxEvidenceWritePort> {
        self.client.clone()
    }

    pub(super) fn shutdown(
        &mut self,
        timeout: Duration,
    ) -> Result<(), SandboxEvidenceShutdownError> {
        self.client.status.mark_stopping();
        match self.client.sender.try_send(WriterMessage::Shutdown) {
            Ok(()) | Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {}
        }
        let Some(thread) = self.thread.as_ref() else {
            return Ok(());
        };
        let deadline = Instant::now().checked_add(timeout).ok_or_else(|| {
            SandboxEvidenceShutdownError::new("deadline", "shutdown deadline overflow")
        })?;
        while !thread.is_finished() {
            if Instant::now() >= deadline {
                return Err(SandboxEvidenceShutdownError::new(
                    "timeout",
                    "sandbox evidence writer did not drain before the shutdown deadline",
                ));
            }
            thread::sleep(Duration::from_millis(2));
        }
        let thread = self.thread.take().ok_or_else(|| {
            SandboxEvidenceShutdownError::new(
                "state",
                "sandbox evidence writer handle disappeared during shutdown",
            )
        })?;
        thread.join().map_err(|_| {
            self.client
                .status
                .record_failure("sandbox evidence writer panicked", true);
            SandboxEvidenceShutdownError::new("panic", "sandbox evidence writer panicked")
        })
    }
}

struct WriterClient {
    sender: SyncSender<WriterMessage>,
    status: Arc<StoreStatus>,
    batch_max_observations: u32,
}

impl SandboxEvidenceWritePort for WriterClient {
    fn try_append_batch(&self, batch: NoInterestEvidenceBatch) -> SandboxEvidenceAdmission {
        let observation_count = batch.observation_count();
        if batch.backing_observation_count() > self.batch_max_observations {
            self.status.rejected_batches.fetch_add(1, Ordering::Relaxed);
            return SandboxEvidenceAdmission::TooLarge {
                observation_count,
                max_observations: self.batch_max_observations,
            };
        }
        if self.status.stopping.load(Ordering::Acquire) {
            self.status.rejected_batches.fetch_add(1, Ordering::Relaxed);
            return SandboxEvidenceAdmission::Closed { observation_count };
        }
        self.status.queue_depth.fetch_add(1, Ordering::AcqRel);
        match self.sender.try_send(WriterMessage::Batch(batch)) {
            Ok(()) => {
                self.status.accepted_batches.fetch_add(1, Ordering::Relaxed);
                self.status
                    .accepted_observations
                    .fetch_add(u64::from(observation_count), Ordering::Relaxed);
                SandboxEvidenceAdmission::Accepted { observation_count }
            }
            Err(TrySendError::Full(_)) => {
                self.status.queue_depth.fetch_sub(1, Ordering::AcqRel);
                self.status.rejected_batches.fetch_add(1, Ordering::Relaxed);
                SandboxEvidenceAdmission::Full { observation_count }
            }
            Err(TrySendError::Disconnected(_)) => {
                self.status.queue_depth.fetch_sub(1, Ordering::AcqRel);
                self.status.rejected_batches.fetch_add(1, Ordering::Relaxed);
                self.status
                    .record_failure("sandbox evidence writer channel is closed", true);
                SandboxEvidenceAdmission::Closed { observation_count }
            }
        }
    }
}

struct WriterOwner {
    config: SandboxEvidenceSqliteConfig,
    receiver: Receiver<WriterMessage>,
    status: Arc<StoreStatus>,
}

impl WriterOwner {
    fn new(
        config: SandboxEvidenceSqliteConfig,
        receiver: Receiver<WriterMessage>,
        status: Arc<StoreStatus>,
    ) -> Self {
        Self {
            config,
            receiver,
            status,
        }
    }

    fn run(self, ready: SyncSender<Result<(), String>>) {
        let connection = self.open_connection();
        let (mut connection, ingest_epoch) = match connection {
            Ok(connection) => connection,
            Err(error) => {
                self.status.record_failure(&error, true);
                if ready.send(Err(error)).is_err() {
                    self.status
                        .record_failure("sandbox evidence startup failure receiver closed", true);
                }
                return;
            }
        };
        self.status.mark_ready(ingest_epoch);
        if ready.send(Ok(())).is_err() {
            self.status
                .record_failure("sandbox evidence readiness receiver closed", true);
            return;
        }
        self.write_loop(&mut connection, ingest_epoch);
        self.status.mark_stopped();
    }

    fn open_connection(&self) -> Result<(Connection, u64), String> {
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let mut connection = Connection::open_with_flags(&self.config.path, flags)
            .map_err(|error| format!("open sandbox evidence database: {error}"))?;
        connection
            .busy_timeout(self.config.busy_timeout)
            .map_err(|error| format!("configure sandbox evidence busy timeout: {error}"))?;
        let ingest_epoch = SandboxSchema::initialize(
            &mut connection,
            self.config.schema_version,
            self.config.synchronous,
            self.config.wal_autocheckpoint_pages,
            self.config.capacity_max_bytes,
        )
        .map_err(|error| format!("initialize sandbox evidence schema: {error}"))?;
        let read_probe = Connection::open_with_flags(
            &self.config.path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|error| format!("open sandbox evidence read probe: {error}"))?;
        read_probe
            .busy_timeout(self.config.busy_timeout)
            .map_err(|error| format!("configure sandbox evidence read probe: {error}"))?;
        SandboxSchema::verify_read_only(&read_probe, self.config.schema_version)
            .map_err(|error| format!("verify sandbox evidence read capability: {error}"))?;
        let retained = connection
            .query_row("SELECT COUNT(*) FROM sandbox_evidence", [], |row| {
                row.get::<_, u64>(0)
            })
            .map_err(|error| format!("count sandbox evidence records: {error}"))?;
        self.status
            .retained_observations
            .store(retained, Ordering::Relaxed);
        Ok((connection, ingest_epoch))
    }

    fn write_loop(&self, connection: &mut Connection, ingest_epoch: u64) {
        loop {
            if self.status.stopping.load(Ordering::Acquire)
                && self.status.queue_depth.load(Ordering::Acquire) == 0
            {
                break;
            }
            let first = match self.receiver.recv_timeout(self.config.flush_interval) {
                Ok(WriterMessage::Batch(batch)) => batch,
                Ok(WriterMessage::Shutdown) => break,
                Err(mpsc::RecvTimeoutError::Timeout)
                    if self.status.stopping.load(Ordering::Acquire) =>
                {
                    break;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            };
            self.status.queue_depth.fetch_sub(1, Ordering::AcqRel);
            let mut batches = Vec::with_capacity(self.config.transaction_max_batches as usize);
            batches.push(first);
            let batch_deadline = Instant::now().checked_add(self.config.flush_interval);
            while batches.len() < self.config.transaction_max_batches as usize {
                let next = if self.status.stopping.load(Ordering::Acquire) {
                    self.receiver.try_recv()
                } else {
                    let remaining = batch_deadline
                        .and_then(|deadline| deadline.checked_duration_since(Instant::now()))
                        .unwrap_or(Duration::ZERO);
                    if remaining.is_zero() {
                        break;
                    }
                    match self.receiver.recv_timeout(remaining) {
                        Ok(batch) => Ok(batch),
                        Err(mpsc::RecvTimeoutError::Timeout) => Err(TryRecvError::Empty),
                        Err(mpsc::RecvTimeoutError::Disconnected) => {
                            Err(TryRecvError::Disconnected)
                        }
                    }
                };
                match next {
                    Ok(WriterMessage::Batch(batch)) => {
                        self.status.queue_depth.fetch_sub(1, Ordering::AcqRel);
                        batches.push(batch);
                    }
                    Ok(WriterMessage::Shutdown) => break,
                    Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
                }
            }
            self.persist_batches(connection, ingest_epoch, &batches);
        }
    }

    fn persist_batches(
        &self,
        connection: &mut Connection,
        ingest_epoch: u64,
        batches: &[NoInterestEvidenceBatch],
    ) {
        match self.persist_transaction(connection, ingest_epoch, batches) {
            Ok(report) => {
                self.status
                    .committed_batches
                    .fetch_add(batches.len() as u64, Ordering::Relaxed);
                self.status
                    .committed_observations
                    .fetch_add(report.inserted, Ordering::Relaxed);
                self.status
                    .duplicate_observations
                    .fetch_add(report.duplicates, Ordering::Relaxed);
                self.status
                    .retained_observations
                    .store(report.retained, Ordering::Relaxed);
                self.status.record_success();
            }
            Err(error) => self.record_batch_failure(batches, error.to_string()),
        }
    }

    fn persist_transaction(
        &self,
        connection: &mut Connection,
        ingest_epoch: u64,
        batches: &[NoInterestEvidenceBatch],
    ) -> Result<TransactionReport, String> {
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let persisted_at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| format!("system clock precedes unix epoch: {error}"))?
            .as_millis()
            .min(u128::from(u64::MAX)) as u64;
        let mut inserted = 0_u64;
        let mut duplicates = 0_u64;
        {
            let mut statement = transaction
                .prepare_cached(
                    "INSERT OR IGNORE INTO sandbox_evidence
                 (ingest_epoch, gateway_id, sb_id, batch_sequence, route_generation,
                  observation_index, observation_kind, persisted_at_ms, payload)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                )
                .map_err(|error| error.to_string())?;
            for batch in batches {
                for index in batch.observation_indices() {
                    let observation = batch.observation(*index).ok_or_else(|| {
                        format!("validated sandbox evidence index {index} became invalid")
                    })?;
                    let (kind, payload) = ObservationCodec::encode(observation);
                    let changed = statement
                        .execute(params![
                            ingest_epoch.to_be_bytes().as_slice(),
                            batch.source().gateway_id(),
                            batch.source().sb_id(),
                            batch.sequence().to_be_bytes().as_slice(),
                            batch.route_generation().to_be_bytes().as_slice(),
                            index,
                            kind,
                            persisted_at_ms.to_be_bytes().as_slice(),
                            payload,
                        ])
                        .map_err(|error| error.to_string())?;
                    if changed == 0 {
                        duplicates = duplicates.saturating_add(1);
                    } else {
                        inserted = inserted.saturating_add(1);
                    }
                }
            }
        }
        let count = transaction
            .query_row("SELECT COUNT(*) FROM sandbox_evidence", [], |row| {
                row.get::<_, u64>(0)
            })
            .map_err(|error| error.to_string())?;
        if count > self.config.retention_max_observations {
            let remove = count - self.config.retention_max_observations;
            transaction
                .execute(
                    "DELETE FROM sandbox_evidence WHERE record_id IN
                 (SELECT record_id FROM sandbox_evidence ORDER BY record_id ASC LIMIT ?1)",
                    [remove],
                )
                .map_err(|error| error.to_string())?;
        }
        let retained = count.min(self.config.retention_max_observations);
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(TransactionReport {
            inserted,
            duplicates,
            retained,
        })
    }

    fn record_batch_failure(&self, batches: &[NoInterestEvidenceBatch], error: String) {
        self.status
            .failed_batches
            .fetch_add(batches.len() as u64, Ordering::Relaxed);
        self.status.record_failure(error, false);
    }
}

struct TransactionReport {
    inserted: u64,
    duplicates: u64,
    retained: u64,
}

enum WriterMessage {
    Batch(NoInterestEvidenceBatch),
    Shutdown,
}
