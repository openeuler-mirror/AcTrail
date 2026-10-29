use super::{
    ResourceStorageSqliteConfig,
    columns::{decode::Decoder, encode::Encoder},
    retention::ResourceRetention,
};
use crate::{
    model::ResourceSample,
    storage::{
        ResourceStorageBackend, ResourceStorageError,
        database::{DatabaseStorageBackend, ResourceKind, ResourceQuery},
    },
};
use rusqlite::{Connection, OptionalExtension, params_from_iter, types::Value};
use std::{
    cell::RefCell,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

impl From<rusqlite::Error> for ResourceStorageError {
    fn from(error: rusqlite::Error) -> Self {
        Self(error.to_string())
    }
}

pub struct ResourceStorageSqliteBackend {
    connection: Connection,
    encoder: Encoder,
    decoder: RefCell<Decoder>,
    write_timeout: Duration,
    retention: ResourceRetention,
    retention_max_age: Option<Duration>,
}

impl ResourceStorageSqliteBackend {
    pub fn open_reader(config: ResourceStorageSqliteConfig) -> Result<Self, ResourceStorageError> {
        let connection =
            Connection::open_with_flags(&config.path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        connection.busy_timeout(Duration::ZERO)?;
        Ok(Self::with_connection(connection, config))
    }

    pub fn open(config: ResourceStorageSqliteConfig) -> Result<Self, ResourceStorageError> {
        let mut connection = Connection::open(&config.path)?;
        connection.busy_timeout(Duration::ZERO)?;
        connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;
        let transaction = connection.transaction()?;
        transaction.execute_batch(include_str!("schema.sql"))?;
        transaction.commit()?;
        Ok(Self::with_connection(connection, config))
    }

    fn with_connection(connection: Connection, config: ResourceStorageSqliteConfig) -> Self {
        Self {
            connection,
            encoder: Encoder::default(),
            decoder: RefCell::new(Decoder::default()),
            write_timeout: Duration::from_millis(config.write_timeout_ms),
            retention_max_age: config
                .retention
                .enabled
                .then(|| Duration::from_secs(config.retention.max_age_seconds)),
            retention: ResourceRetention::new(config.retention),
        }
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

    fn table(kind: ResourceKind) -> &'static str {
        match kind {
            ResourceKind::Host => "resource_host_samples",
            ResourceKind::Workload => "resource_workload_samples",
        }
    }

    /// Only filters that are set appear in SQL, so the trace/boot indexes match.
    /// None means there can be no matches, including a reader of an old schema.
    fn filter(
        &self,
        query: &ResourceQuery,
    ) -> Result<Option<(String, Vec<Value>)>, ResourceStorageError> {
        let (since, until) = Self::bounds(query)?;
        let exists = self
            .connection
            .prepare_cached("SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1")?
            .query_row([Self::table(query.kind)], |_| Ok(()))
            .optional()?
            .is_some();
        if !exists
            || (query.kind == ResourceKind::Host && query.trace_id.is_some())
            || (query.kind == ResourceKind::Workload && query.host_boot_id.is_some())
        {
            return Ok(None);
        }
        let mut filter = "observed_at_ns BETWEEN ? AND ?".to_owned();
        let mut values = vec![Value::Integer(since), Value::Integer(until)];
        if let Some(boot) = query.host_boot_id {
            let key = self
                .connection
                .prepare_cached("SELECT boot_key FROM resource_host_boots WHERE boot_id=?1")?
                .query_row([boot.as_bytes().as_slice()], |row| row.get::<_, i64>(0))
                .optional()?;
            let Some(key) = key else { return Ok(None) };
            filter.push_str(" AND boot_key=?");
            values.push(Value::Integer(key));
        }
        if let Some(trace) = query.trace_id {
            filter.push_str(" AND trace_id=?");
            values.push(Value::Integer(trace.get() as i64));
        }
        values.push(Value::Integer(i64::from(query.limit)));
        Ok(Some((filter, values)))
    }

    fn read(
        &self,
        query: &ResourceQuery,
        descending: bool,
        visit: &mut dyn FnMut(ResourceSample),
    ) -> Result<u32, ResourceStorageError> {
        let Some((filter, values)) = self.filter(query)? else {
            return Ok(0);
        };
        let table = Self::table(query.kind);
        let columns = Decoder::columns(query.kind);
        let order = if descending { "DESC" } else { "ASC" };
        let mut statement = self.connection.prepare_cached(&format!(
            "SELECT {columns} FROM {table} WHERE {filter} ORDER BY observed_at_ns {order}, id {order} LIMIT ?"
        ))?;
        let mut rows = statement.query(params_from_iter(values))?;
        let mut visited = 0;
        while let Some(row) = rows.next()? {
            let sample = self
                .decoder
                .borrow_mut()
                .sample(&self.connection, query.kind, row)?;
            visit(sample);
            visited += 1;
        }
        Ok(visited)
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
        let result = (|| {
            self.connection.busy_timeout(timeout)?;
            let transaction = self.connection.transaction()?;
            self.encoder
                .write(&transaction, samples, started, timeout)?;
            transaction.commit()?;
            Ok(())
        })();
        if result.is_err() {
            self.encoder.clear();
            return result;
        }
        match self.retention.sweep(&self.connection) {
            Ok(false) => {}
            Ok(true) => {
                self.encoder.clear_profiles_and_boots();
                self.decoder.get_mut().clear_profiles_and_boots();
            }
            Err(error) => {
                // Reclamation may have partially succeeded before the error.
                self.encoder.clear_profiles_and_boots();
                self.decoder.get_mut().clear_profiles_and_boots();
                tracing::warn!(%error, "resource retention failed");
            }
        }
        Ok(())
    }
}

impl DatabaseStorageBackend for ResourceStorageSqliteBackend {
    fn query(&self, query: &ResourceQuery) -> Result<Vec<ResourceSample>, ResourceStorageError> {
        let mut samples = Vec::new();
        self.read(query, true, &mut |sample| samples.push(sample))?;
        Ok(samples)
    }

    fn scan_ascending(
        &self,
        query: &ResourceQuery,
        visit: &mut dyn FnMut(ResourceSample),
    ) -> Result<u32, ResourceStorageError> {
        self.read(query, false, visit)
    }

    fn retention_max_age(&self) -> Option<Duration> {
        self.retention_max_age
    }

    fn delete(&mut self, query: &ResourceQuery) -> Result<usize, ResourceStorageError> {
        let Some((filter, values)) = self.filter(query)? else {
            return Ok(0);
        };
        let table = Self::table(query.kind);
        let mut statement = self.connection.prepare_cached(&format!(
            "DELETE FROM {table} WHERE id IN (SELECT id FROM {table} WHERE {filter} ORDER BY observed_at_ns LIMIT ?)"
        ))?;
        Ok(statement.execute(params_from_iter(values))?)
    }
}
