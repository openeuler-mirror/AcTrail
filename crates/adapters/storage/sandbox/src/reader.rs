use std::path::PathBuf;
use std::time::Duration;

use rusqlite::{Connection, OpenFlags};
use sandbox_evidence_store::{
    SandboxEvidenceReadError, SandboxEvidenceReadPort, SandboxEvidenceSource, StoredSandboxEvidence,
};

use crate::codec::ObservationCodec;
use crate::schema::SandboxSchema;

pub struct SandboxEvidenceSqliteReader {
    path: PathBuf,
    schema_version: u32,
    busy_timeout: Duration,
    read_limit_max: u32,
}

impl SandboxEvidenceSqliteReader {
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

    fn open(&self) -> Result<Connection, SandboxEvidenceReadError> {
        let connection = Connection::open_with_flags(
            &self.path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|error| SandboxEvidenceReadError::new("open", error.to_string()))?;
        connection
            .busy_timeout(self.busy_timeout)
            .map_err(|error| SandboxEvidenceReadError::new("busy_timeout", error.to_string()))?;
        SandboxSchema::verify_read_only(&connection, self.schema_version)
            .map_err(|error| SandboxEvidenceReadError::new("schema", error.to_string()))?;
        Ok(connection)
    }
}

impl SandboxEvidenceReadPort for SandboxEvidenceSqliteReader {
    fn recent(&self, limit: u32) -> Result<Vec<StoredSandboxEvidence>, SandboxEvidenceReadError> {
        if limit == 0 || limit > self.read_limit_max {
            return Err(SandboxEvidenceReadError::new(
                "limit",
                format!(
                    "sandbox evidence read limit must be within 1..={}",
                    self.read_limit_max
                ),
            ));
        }
        let connection = self.open()?;
        let mut statement = connection
            .prepare(
                "SELECT record_id, ingest_epoch, gateway_id, sb_id, batch_sequence, route_generation,
                        observation_index, observation_kind, persisted_at_ms, payload
                 FROM sandbox_evidence ORDER BY record_id DESC LIMIT ?1",
            )
            .map_err(|error| SandboxEvidenceReadError::new("prepare", error.to_string()))?;
        let rows = statement
            .query_map([limit], |row| {
                Ok((
                    row.get::<_, u64>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, u32>(2)?,
                    row.get::<_, u32>(3)?,
                    row.get::<_, Vec<u8>>(4)?,
                    row.get::<_, Vec<u8>>(5)?,
                    row.get::<_, u32>(6)?,
                    row.get::<_, u8>(7)?,
                    row.get::<_, Vec<u8>>(8)?,
                    row.get::<_, Vec<u8>>(9)?,
                ))
            })
            .map_err(|error| SandboxEvidenceReadError::new("query", error.to_string()))?;
        let mut records = Vec::with_capacity(limit as usize);
        for row in rows {
            let (
                record_id,
                ingest_epoch,
                gateway_id,
                sb_id,
                sequence,
                generation,
                index,
                kind,
                persisted,
                payload,
            ) = row.map_err(|error| SandboxEvidenceReadError::new("row", error.to_string()))?;
            records.push(StoredSandboxEvidence {
                record_id,
                ingest_epoch: decode_u64(&ingest_epoch)?,
                source: SandboxEvidenceSource::new(gateway_id, sb_id).map_err(|error| {
                    SandboxEvidenceReadError::new("source", format!("{error:?}"))
                })?,
                batch_sequence: decode_u64(&sequence)?,
                route_generation: decode_u64(&generation)?,
                observation_index: index,
                persisted_at_ms: decode_u64(&persisted)?,
                observation: ObservationCodec::decode(kind, &payload)
                    .map_err(|error| SandboxEvidenceReadError::new("payload", error))?,
            });
        }
        Ok(records)
    }
}

fn decode_u64(bytes: &[u8]) -> Result<u64, SandboxEvidenceReadError> {
    bytes
        .try_into()
        .map(u64::from_be_bytes)
        .map_err(|_| SandboxEvidenceReadError::new("integer", "invalid stored u64 width"))
}
