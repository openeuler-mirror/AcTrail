use super::{ResourceStorageSqliteConfig, retention::ResourceRetention};
use crate::{
    model::ResourceSample,
    storage::{
        ResourceStorageBackend, ResourceStorageError,
        database::{DatabaseStorageBackend, ResourceKind, ResourceQuery},
    },
};
use rusqlite::{Connection, params};
impl From<rusqlite::Error> for ResourceStorageError {
    fn from(error: rusqlite::Error) -> Self {
        Self(error.to_string())
    }
}
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
pub struct ResourceStorageSqliteBackend {
    connection: Connection,
    write_timeout: Duration,
    retention: ResourceRetention,
}
impl ResourceStorageSqliteBackend {
    pub fn open_reader(config: ResourceStorageSqliteConfig) -> Result<Self, ResourceStorageError> {
        let connection =
            Connection::open_with_flags(&config.path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        connection.busy_timeout(Duration::ZERO)?;
        Ok(Self {
            connection,
            write_timeout: Duration::from_millis(config.write_timeout_ms),
            retention: ResourceRetention::new(config.retention),
        })
    }

    pub fn open(config: ResourceStorageSqliteConfig) -> Result<Self, ResourceStorageError> {
        let connection = Connection::open(&config.path)?;
        connection.busy_timeout(Duration::ZERO)?;
        connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;
        connection.execute_batch(include_str!("schema.sql"))?;
        Ok(Self {
            connection,
            write_timeout: Duration::from_millis(config.write_timeout_ms),
            retention: ResourceRetention::new(config.retention),
        })
    }
    pub(super) fn timestamp(at: SystemTime) -> Result<i64, ResourceStorageError> {
        at.duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|d| i64::try_from(d.as_nanos()).ok())
            .ok_or_else(|| ResourceStorageError("resource timestamp outside SQLite range".into()))
    }
    fn bounds(query: &ResourceQuery) -> Result<(i64, i64), ResourceStorageError> {
        let since = query.since.map(Self::timestamp).transpose()?.unwrap_or(0);
        let until = query
            .until
            .map(Self::timestamp)
            .transpose()?
            .unwrap_or(i64::MAX);
        if since > until || query.limit == 0 {
            return Err(ResourceStorageError(
                "invalid resource query range or limit".into(),
            ));
        }
        Ok((since, until))
    }
}
impl ResourceStorageBackend for ResourceStorageSqliteBackend {
    fn write(
        &mut self,
        samples: &[ResourceSample],
        timeout: Duration,
    ) -> Result<(), ResourceStorageError> {
        let timeout = timeout.min(self.write_timeout);
        let started = Instant::now();
        self.connection.busy_timeout(timeout)?;
        let transaction = self.connection.transaction()?;
        {
            let mut insert = transaction.prepare_cached("INSERT INTO resource_samples(kind,observed_at_ns,host_boot_id,trace_id,payload) VALUES (?1,?2,?3,?4,?5)")?;
            for sample in samples {
                if started.elapsed() >= timeout {
                    return Err(ResourceStorageError("resource SQLite batch timeout".into()));
                }
                let (kind, boot, trace) = match sample {
                    ResourceSample::Host(s) => ("host", Some(s.host_boot_id.to_string()), None),
                    ResourceSample::Workload(s) => {
                        ("workload", None, Some(s.trace_id.get() as i64))
                    }
                };
                insert.execute(params![
                    kind,
                    Self::timestamp(sample.observed_at())?,
                    boot,
                    trace,
                    serde_json::to_string(sample)?
                ])?;
            }
        }
        transaction.commit()?;
        if let Err(error) = self.retention.sweep(&self.connection) {
            tracing::warn!(%error, "resource retention failed");
        }
        Ok(())
    }
}
impl DatabaseStorageBackend for ResourceStorageSqliteBackend {
    fn query(&self, query: &ResourceQuery) -> Result<Vec<ResourceSample>, ResourceStorageError> {
        let (since, until) = Self::bounds(query)?;
        let kind = match query.kind {
            ResourceKind::Host => "host",
            ResourceKind::Workload => "workload",
        };
        let mut statement = self.connection.prepare("SELECT payload FROM resource_samples WHERE kind=?1 AND observed_at_ns BETWEEN ?2 AND ?3 AND (?4 IS NULL OR host_boot_id=?4) AND (?5 IS NULL OR trace_id=?5) ORDER BY observed_at_ns DESC, id DESC LIMIT ?6")?;
        let rows = statement.query_map(
            params![
                kind,
                since,
                until,
                query.host_boot_id.map(|id| id.to_string()),
                query.trace_id.map(|id| id.get() as i64),
                query.limit
            ],
            |row| row.get::<_, String>(0),
        )?;
        rows.map(|row| Ok(serde_json::from_str(&row?)?)).collect()
    }
    fn delete(&mut self, query: &ResourceQuery) -> Result<usize, ResourceStorageError> {
        let (since, until) = Self::bounds(query)?;
        let kind = match query.kind {
            ResourceKind::Host => "host",
            ResourceKind::Workload => "workload",
        };
        Ok(self.connection.execute("DELETE FROM resource_samples WHERE id IN (SELECT id FROM resource_samples WHERE kind=?1 AND observed_at_ns BETWEEN ?2 AND ?3 AND (?4 IS NULL OR host_boot_id=?4) AND (?5 IS NULL OR trace_id=?5) ORDER BY observed_at_ns LIMIT ?6)",params![kind,since,until,query.host_boot_id.map(|id|id.to_string()),query.trace_id.map(|id|id.get() as i64),query.limit])?)
    }
}
