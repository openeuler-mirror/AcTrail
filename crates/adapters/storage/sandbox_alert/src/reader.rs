use std::path::PathBuf;
use std::time::Duration;

use rusqlite::{Connection, OpenFlags};
use sandbox_alert_store::{
    SandboxAlertReadError, SandboxAlertReadPort, SandboxAlertRecord, SandboxAlertSource,
    StoredSandboxAlert,
};

use crate::codec::AlertCodec;
use crate::schema::SandboxAlertSchema;

pub struct SandboxAlertSqliteReader {
    path: PathBuf,
    schema_version: u32,
    busy_timeout: Duration,
    read_limit_max: u32,
}

impl SandboxAlertSqliteReader {
    pub(crate) fn new(
        path: PathBuf,
        schema_version: u32,
        busy_timeout: Duration,
        read_limit_max: u32,
    ) -> Self {
        Self {
            path,
            schema_version,
            busy_timeout,
            read_limit_max,
        }
    }

    fn open(&self) -> Result<Connection, SandboxAlertReadError> {
        let connection = Connection::open_with_flags(
            &self.path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|error| SandboxAlertReadError::new("open", error.to_string()))?;
        connection
            .busy_timeout(self.busy_timeout)
            .map_err(|error| SandboxAlertReadError::new("busy_timeout", error.to_string()))?;
        SandboxAlertSchema::verify_read_only(&connection, self.schema_version)
            .map_err(|error| SandboxAlertReadError::new("schema", error.to_string()))?;
        Ok(connection)
    }
}

impl SandboxAlertReadPort for SandboxAlertSqliteReader {
    fn recent(&self, limit: u32) -> Result<Vec<StoredSandboxAlert>, SandboxAlertReadError> {
        if limit == 0 || limit > self.read_limit_max {
            return Err(SandboxAlertReadError::new(
                "limit",
                format!(
                    "sandbox alert read limit must be within 1..={}",
                    self.read_limit_max
                ),
            ));
        }
        let connection = self.open()?;
        let mut statement = connection
            .prepare(
                "SELECT alert_id, ingest_epoch, gateway_id, sb_id, batch_sequence,
                        observation_index, alert_kind, detected_at_ms, persisted_at_ms, payload
                 FROM sandbox_alerts ORDER BY alert_id DESC LIMIT ?1",
            )
            .map_err(|error| SandboxAlertReadError::new("prepare", error.to_string()))?;
        let rows = statement
            .query_map([limit], |row| {
                Ok((
                    row.get::<_, u64>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, u32>(2)?,
                    row.get::<_, u32>(3)?,
                    row.get::<_, Vec<u8>>(4)?,
                    row.get::<_, u32>(5)?,
                    row.get::<_, u8>(6)?,
                    row.get::<_, Vec<u8>>(7)?,
                    row.get::<_, Vec<u8>>(8)?,
                    row.get::<_, Vec<u8>>(9)?,
                ))
            })
            .map_err(|error| SandboxAlertReadError::new("query", error.to_string()))?;
        let mut alerts = Vec::with_capacity(limit as usize);
        for row in rows {
            let (id, epoch, gateway, sb, sequence, index, kind, detected, persisted, payload) =
                row.map_err(|error| SandboxAlertReadError::new("row", error.to_string()))?;
            let detected_at_ms = decode_u64(&detected)?;
            let source = SandboxAlertSource::new(gateway, sb)
                .map_err(|error| SandboxAlertReadError::new("source", format!("{error:?}")))?;
            let kind = AlertCodec::decode(kind, detected_at_ms, &payload)
                .map_err(|error| SandboxAlertReadError::new("payload", error))?;
            alerts.push(StoredSandboxAlert {
                alert_id: id,
                ingest_epoch: decode_u64(&epoch)?,
                persisted_at_ms: decode_u64(&persisted)?,
                alert: SandboxAlertRecord::new(source, decode_u64(&sequence)?, index, kind),
            });
        }
        Ok(alerts)
    }
}

fn decode_u64(bytes: &[u8]) -> Result<u64, SandboxAlertReadError> {
    bytes
        .try_into()
        .map(u64::from_be_bytes)
        .map_err(|_| SandboxAlertReadError::new("integer", "invalid stored u64 width"))
}
