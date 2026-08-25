use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OpenFlags, TransactionBehavior, params};
use sandbox_alert_store::{
    SandboxAlertAdmission, SandboxAlertCommitPort, SandboxAlertRecord, SandboxAlertShutdownError,
    SandboxAlertWritePort, StoredSandboxAlert,
};

use crate::codec::AlertCodec;
use crate::config::SandboxAlertSqliteConfig;
use crate::schema::SandboxAlertSchema;
use crate::status::StoreStatus;

pub(super) struct SandboxAlertWriter {
    client: Arc<WriterClient>,
    thread: Option<JoinHandle<()>>,
}

impl SandboxAlertWriter {
    pub(super) fn start(
        config: SandboxAlertSqliteConfig,
        status: Arc<StoreStatus>,
        commit_port: Arc<dyn SandboxAlertCommitPort>,
    ) -> Result<Self, String> {
        let queue_capacity = usize::try_from(config.writer_queue_capacity)
            .map_err(|error| format!("sandbox alert writer queue overflow: {error}"))?;
        let (sender, receiver) = mpsc::sync_channel(queue_capacity);
        let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
        let worker_status = Arc::clone(&status);
        let thread = thread::Builder::new()
            .name("actrail-sandbox-alert-store".to_string())
            .stack_size(config.writer_thread_stack_bytes)
            .spawn(move || {
                WriterOwner::new(config, receiver, worker_status, commit_port).run(ready_sender)
            })
            .map_err(|error| format!("spawn sandbox alert writer: {error}"))?;
        match ready_receiver.recv() {
            Ok(Ok(())) => Ok(Self {
                client: Arc::new(WriterClient { sender, status }),
                thread: Some(thread),
            }),
            Ok(Err(error)) => {
                if thread.join().is_err() {
                    return Err(format!(
                        "{error}; sandbox alert writer panicked during failed startup"
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
                    "sandbox alert writer readiness channel closed: {error}{join_message}"
                ))
            }
        }
    }

    pub(super) fn port(&self) -> Arc<dyn SandboxAlertWritePort> {
        self.client.clone()
    }

    pub(super) fn shutdown(&mut self, timeout: Duration) -> Result<(), SandboxAlertShutdownError> {
        self.client.status.mark_stopping();
        let Some(thread) = self.thread.as_ref() else {
            return Ok(());
        };
        let deadline = Instant::now().checked_add(timeout).ok_or_else(|| {
            SandboxAlertShutdownError::new("deadline", "shutdown deadline overflow")
        })?;
        while !thread.is_finished() {
            if Instant::now() >= deadline {
                return Err(SandboxAlertShutdownError::new(
                    "timeout",
                    "sandbox alert writer did not drain before the shutdown deadline",
                ));
            }
            thread::sleep(Duration::from_millis(2));
        }
        let thread = self.thread.take().ok_or_else(|| {
            SandboxAlertShutdownError::new(
                "state",
                "sandbox alert writer handle disappeared during shutdown",
            )
        })?;
        thread.join().map_err(|_| {
            self.client
                .status
                .record_failure("sandbox alert writer panicked", true);
            SandboxAlertShutdownError::new("panic", "sandbox alert writer panicked")
        })
    }
}

struct WriterClient {
    sender: SyncSender<SandboxAlertRecord>,
    status: Arc<StoreStatus>,
}

impl SandboxAlertWritePort for WriterClient {
    fn try_append(&self, alert: SandboxAlertRecord) -> SandboxAlertAdmission {
        if self.status.stopping.load(Ordering::Acquire) {
            self.status.rejected_alerts.fetch_add(1, Ordering::Relaxed);
            return SandboxAlertAdmission::Closed;
        }
        self.status.queue_depth.fetch_add(1, Ordering::AcqRel);
        match self.sender.try_send(alert) {
            Ok(()) => {
                self.status.accepted_alerts.fetch_add(1, Ordering::Relaxed);
                SandboxAlertAdmission::Accepted
            }
            Err(TrySendError::Full(_)) => {
                self.status.queue_depth.fetch_sub(1, Ordering::AcqRel);
                self.status.rejected_alerts.fetch_add(1, Ordering::Relaxed);
                SandboxAlertAdmission::Full
            }
            Err(TrySendError::Disconnected(_)) => {
                self.status.queue_depth.fetch_sub(1, Ordering::AcqRel);
                self.status.rejected_alerts.fetch_add(1, Ordering::Relaxed);
                self.status
                    .record_failure("sandbox alert writer channel is closed", true);
                SandboxAlertAdmission::Closed
            }
        }
    }
}

struct WriterOwner {
    config: SandboxAlertSqliteConfig,
    receiver: Receiver<SandboxAlertRecord>,
    status: Arc<StoreStatus>,
    commit_port: Arc<dyn SandboxAlertCommitPort>,
}

impl WriterOwner {
    fn new(
        config: SandboxAlertSqliteConfig,
        receiver: Receiver<SandboxAlertRecord>,
        status: Arc<StoreStatus>,
        commit_port: Arc<dyn SandboxAlertCommitPort>,
    ) -> Self {
        Self {
            config,
            receiver,
            status,
            commit_port,
        }
    }

    fn run(self, ready: SyncSender<Result<(), String>>) {
        let (mut connection, ingest_epoch) = match self.open_connection() {
            Ok(connection) => connection,
            Err(error) => {
                self.status.record_failure(&error, true);
                let _ = ready.send(Err(error));
                return;
            }
        };
        self.status.mark_ready(ingest_epoch);
        if ready.send(Ok(())).is_err() {
            self.status
                .record_failure("sandbox alert readiness receiver closed", true);
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
            .map_err(|error| format!("open sandbox alert database: {error}"))?;
        connection
            .busy_timeout(self.config.busy_timeout)
            .map_err(|error| format!("configure sandbox alert busy timeout: {error}"))?;
        let ingest_epoch = SandboxAlertSchema::initialize(
            &mut connection,
            self.config.schema_version,
            self.config.synchronous,
            self.config.wal_autocheckpoint_pages,
            self.config.capacity_max_bytes,
        )
        .map_err(|error| format!("initialize sandbox alert schema: {error}"))?;
        let read_probe = Connection::open_with_flags(
            &self.config.path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|error| format!("open sandbox alert read probe: {error}"))?;
        read_probe
            .busy_timeout(self.config.busy_timeout)
            .map_err(|error| format!("configure sandbox alert read probe: {error}"))?;
        SandboxAlertSchema::verify_read_only(&read_probe, self.config.schema_version)
            .map_err(|error| format!("verify sandbox alert read capability: {error}"))?;
        let retained = connection
            .query_row("SELECT COUNT(*) FROM sandbox_alerts", [], |row| {
                row.get::<_, u64>(0)
            })
            .map_err(|error| format!("count sandbox alerts: {error}"))?;
        self.status
            .retained_alerts
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
                Ok(alert) => alert,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            };
            self.status.queue_depth.fetch_sub(1, Ordering::AcqRel);
            let mut alerts = Vec::with_capacity(self.config.transaction_max_alerts as usize);
            alerts.push(first);
            while alerts.len() < self.config.transaction_max_alerts as usize {
                match self.receiver.try_recv() {
                    Ok(alert) => {
                        self.status.queue_depth.fetch_sub(1, Ordering::AcqRel);
                        alerts.push(alert);
                    }
                    Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
                }
            }
            self.persist_alerts(connection, ingest_epoch, &alerts);
        }
    }

    fn persist_alerts(
        &self,
        connection: &mut Connection,
        ingest_epoch: u64,
        alerts: &[SandboxAlertRecord],
    ) {
        match self.persist_transaction(connection, ingest_epoch, alerts) {
            Ok(report) => {
                self.status
                    .committed_alerts
                    .fetch_add(report.inserted, Ordering::Relaxed);
                self.status
                    .duplicate_alerts
                    .fetch_add(report.duplicates, Ordering::Relaxed);
                self.status
                    .retained_alerts
                    .store(report.retained, Ordering::Relaxed);
                self.status.record_success();
                for alert in report.committed {
                    self.commit_port.committed(alert);
                }
            }
            Err(error) => {
                self.status
                    .failed_alerts
                    .fetch_add(alerts.len() as u64, Ordering::Relaxed);
                self.status.record_failure(error, false);
            }
        }
    }

    fn persist_transaction(
        &self,
        connection: &mut Connection,
        ingest_epoch: u64,
        alerts: &[SandboxAlertRecord],
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
        let mut committed = Vec::with_capacity(alerts.len());
        {
            let mut statement = transaction
                .prepare_cached(
                    "INSERT OR IGNORE INTO sandbox_alerts
                     (ingest_epoch, gateway_id, sb_id, batch_sequence, observation_index,
                      alert_kind, detected_at_ms, persisted_at_ms, payload)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                )
                .map_err(|error| error.to_string())?;
            for alert in alerts {
                let source = alert.source();
                let (kind, payload) = AlertCodec::encode(alert.kind());
                let changed = statement
                    .execute(params![
                        ingest_epoch.to_be_bytes().as_slice(),
                        source.gateway_id(),
                        source.sb_id(),
                        alert.batch_sequence().to_be_bytes().as_slice(),
                        alert.observation_index(),
                        kind,
                        alert.detected_at_ms().to_be_bytes().as_slice(),
                        persisted_at_ms.to_be_bytes().as_slice(),
                        payload,
                    ])
                    .map_err(|error| error.to_string())?;
                if changed == 0 {
                    duplicates = duplicates.saturating_add(1);
                } else {
                    inserted = inserted.saturating_add(1);
                    committed.push(StoredSandboxAlert {
                        alert_id: transaction.last_insert_rowid() as u64,
                        ingest_epoch,
                        persisted_at_ms,
                        alert: *alert,
                    });
                }
            }
        }
        let count = transaction
            .query_row("SELECT COUNT(*) FROM sandbox_alerts", [], |row| {
                row.get::<_, u64>(0)
            })
            .map_err(|error| error.to_string())?;
        if count > self.config.retention_max_alerts {
            transaction
                .execute(
                    "DELETE FROM sandbox_alerts WHERE alert_id IN
                     (SELECT alert_id FROM sandbox_alerts ORDER BY alert_id ASC LIMIT ?1)",
                    [count - self.config.retention_max_alerts],
                )
                .map_err(|error| error.to_string())?;
        }
        let retained = count.min(self.config.retention_max_alerts);
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(TransactionReport {
            inserted,
            duplicates,
            retained,
            committed,
        })
    }
}

struct TransactionReport {
    inserted: u64,
    duplicates: u64,
    retained: u64,
    committed: Vec<StoredSandboxAlert>,
}
