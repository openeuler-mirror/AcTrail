//! Read-only access to the catalog, for the coordinator's queries and for Web.

use std::path::Path;
use std::time::Duration;

use checkpoint_core::model::{
    CheckpointRecord, FilterMode, OperationId, OperationRecord, TraceRecord,
};
use checkpoint_core::ports::{CatalogError, CatalogReader};
use model_core::ids::TraceId;
use rusqlite::{Connection, OpenFlags};

use crate::{checkpoints, db, operations, traces};

pub struct CheckpointSqliteReader {
    connection: Connection,
}

/// The records of one Trace read from a single catalog snapshot.
pub struct CheckpointHistory {
    pub trace: Option<TraceRecord>,
    pub checkpoints: Vec<CheckpointRecord>,
    pub operations: Vec<OperationRecord>,
}

impl CheckpointSqliteReader {
    /// Waits up to `busy_timeout` while the daemon holds a write lock.
    pub fn open(path: &Path, busy_timeout: Duration) -> Result<Self, CatalogError> {
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(db)?;
        connection.busy_timeout(busy_timeout).map_err(db)?;
        Ok(Self { connection })
    }

    /// The next Trace ID after every ID retained in this catalog.
    pub fn next_trace_id_seed(&self) -> Result<u64, CatalogError> {
        traces::next_id_seed(&self.connection).map_err(db)
    }

    /// Reads related history together, so concurrent saves and restores cannot split its view.
    pub fn history(&mut self, trace_id: TraceId) -> Result<CheckpointHistory, CatalogError> {
        let transaction = self.connection.transaction().map_err(db)?;
        let history = CheckpointHistory {
            trace: traces::get(&transaction, trace_id).map_err(db)?,
            checkpoints: checkpoints::for_trace(&transaction, trace_id).map_err(db)?,
            operations: operations::for_trace(&transaction, trace_id).map_err(db)?,
        };
        transaction.commit().map_err(db)?;
        Ok(history)
    }

    pub fn trace(&self, trace_id: TraceId) -> Result<Option<TraceRecord>, CatalogError> {
        traces::get(&self.connection, trace_id).map_err(db)
    }

    pub fn checkpoints(
        &self,
        trace_id: TraceId,
        filter_mode: &FilterMode,
    ) -> Result<Vec<CheckpointRecord>, CatalogError> {
        checkpoints::matching(&self.connection, trace_id, filter_mode).map_err(db)
    }

    pub fn operations(&self, trace_id: TraceId) -> Result<Vec<OperationRecord>, CatalogError> {
        operations::for_trace(&self.connection, trace_id).map_err(db)
    }

    pub fn operation(
        &self,
        operation_id: OperationId,
    ) -> Result<Option<OperationRecord>, CatalogError> {
        operations::get(&self.connection, operation_id).map_err(db)
    }
}

impl CatalogReader for CheckpointSqliteReader {
    fn trace(&self, trace_id: TraceId) -> Result<Option<TraceRecord>, CatalogError> {
        Self::trace(self, trace_id)
    }

    fn checkpoints(
        &self,
        trace_id: TraceId,
        filter_mode: &FilterMode,
    ) -> Result<Vec<CheckpointRecord>, CatalogError> {
        Self::checkpoints(self, trace_id, filter_mode)
    }

    fn operations(&self, trace_id: TraceId) -> Result<Vec<OperationRecord>, CatalogError> {
        Self::operations(self, trace_id)
    }

    fn operation(
        &self,
        operation_id: OperationId,
    ) -> Result<Option<OperationRecord>, CatalogError> {
        Self::operation(self, operation_id)
    }
}
