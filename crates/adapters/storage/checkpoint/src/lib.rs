//! SQLite checkpoint catalog. The daemon writes through `CheckpointSqliteCatalog`; Web reads
//! through `CheckpointSqliteReader` while the daemon keeps writing.

mod checkpoints;
mod codec;
mod operations;
mod reader;
mod schema;
mod traces;

use std::fs;
use std::path::Path;
use std::time::SystemTime;

use checkpoint_core::model::{
    Boundary, CheckpointId, CheckpointRecord, Disconnected, Failure, NewCheckpoint, NewOperation,
    OperationId, OperationKind, OperationOutcome, OperationRecord, TraceRecord,
};
use checkpoint_core::ports::{CatalogError, CheckpointCatalog};
use model_core::ids::TraceId;
use rusqlite::Connection;

pub use reader::{CheckpointHistory, CheckpointSqliteReader};

use crate::operations::Finish;

pub struct CheckpointSqliteCatalog {
    connection: Connection,
}

impl CheckpointSqliteCatalog {
    /// Opens the catalog at `path`, creating the file and its tables on first use.
    pub fn open(path: &Path) -> Result<Self, CatalogError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| CatalogError::new(format!("{}: {error}", parent.display())))?;
        }
        let mut connection = Connection::open(path).map_err(db)?;
        schema::initialize(&mut connection)
            .map_err(|message| CatalogError::new(format!("{}: {message}", path.display())))?;
        Ok(Self { connection })
    }

    fn operation_of_kind(
        &self,
        operation_id: OperationId,
        kind: OperationKind,
    ) -> Result<TraceId, CatalogError> {
        match operations::kind(&self.connection, operation_id).map_err(db)? {
            Some((trace_id, found)) if found == kind => Ok(trace_id),
            Some((_, found)) => Err(CatalogError::new(format!(
                "{operation_id} is a {} operation, not a {}",
                found.as_str(),
                kind.as_str()
            ))),
            None => Err(unknown(operation_id)),
        }
    }
}

impl CheckpointCatalog for CheckpointSqliteCatalog {
    fn create_trace(&mut self, trace: &TraceRecord) -> Result<(), CatalogError> {
        traces::insert(&self.connection, trace).map_err(db)
    }

    fn trace(&self, trace_id: TraceId) -> Result<Option<TraceRecord>, CatalogError> {
        traces::get(&self.connection, trace_id).map_err(db)
    }

    fn mark_trace_ended(
        &mut self,
        trace_id: TraceId,
        ended_at: SystemTime,
    ) -> Result<(), CatalogError> {
        updated_one(
            traces::mark_ended(&self.connection, trace_id, ended_at),
            trace_id,
        )
    }

    fn record_disconnect(
        &mut self,
        trace_id: TraceId,
        disconnected: &Disconnected,
    ) -> Result<(), CatalogError> {
        updated_one(
            traces::record_disconnect(&self.connection, trace_id, disconnected),
            trace_id,
        )
    }

    fn unsettled_traces(&self) -> Result<Vec<TraceId>, CatalogError> {
        traces::unsettled(&self.connection).map_err(db)
    }

    fn create_operation(&mut self, operation: &NewOperation) -> Result<OperationId, CatalogError> {
        operations::insert(&self.connection, operation).map_err(db)
    }

    fn start_operation(
        &mut self,
        operation_id: OperationId,
        started_at: SystemTime,
        boundary: Option<&Boundary>,
    ) -> Result<(), CatalogError> {
        updated_one(
            operations::start(&self.connection, operation_id, started_at, boundary),
            operation_id,
        )
    }

    fn finish_save(
        &mut self,
        operation_id: OperationId,
        finished_at: SystemTime,
        outcome: OperationOutcome,
        failure: Option<&Failure>,
        checkpoint: Option<&NewCheckpoint>,
    ) -> Result<Option<CheckpointId>, CatalogError> {
        let trace_id = self.operation_of_kind(operation_id, OperationKind::Save)?;
        let transaction = self.connection.transaction().map_err(db)?;
        let created = checkpoint
            .map(|checkpoint| {
                checkpoints::insert(
                    &transaction,
                    trace_id,
                    operation_id,
                    finished_at,
                    checkpoint,
                )
            })
            .transpose()
            .map_err(db)?;
        let finish = Finish {
            finished_at,
            outcome,
            failure,
            created_checkpoint: created,
            invalidated: &[],
        };
        updated_one(
            operations::finish(&transaction, operation_id, &finish),
            operation_id,
        )?;
        transaction.commit().map_err(db)?;
        Ok(created)
    }

    fn finish_restore(
        &mut self,
        operation_id: OperationId,
        finished_at: SystemTime,
        outcome: OperationOutcome,
        failure: Option<&Failure>,
        invalidated: &[Vec<u8>],
    ) -> Result<(), CatalogError> {
        let trace_id = self.operation_of_kind(operation_id, OperationKind::Restore)?;
        let transaction = self.connection.transaction().map_err(db)?;
        let invalidated =
            checkpoints::invalidate(&transaction, trace_id, operation_id, invalidated)
                .map_err(db)?;
        let finish = Finish {
            finished_at,
            outcome,
            failure,
            created_checkpoint: None,
            invalidated: &invalidated,
        };
        updated_one(
            operations::finish(&transaction, operation_id, &finish),
            operation_id,
        )?;
        transaction.commit().map_err(db)
    }

    fn mark_checkpoint_deleted(
        &mut self,
        trace_id: TraceId,
        checkpoint_id: CheckpointId,
        deleted_at: SystemTime,
    ) -> Result<Vec<u8>, CatalogError> {
        checkpoints::mark_deleted(&self.connection, trace_id, checkpoint_id, deleted_at)
            .map_err(db)?
            .ok_or_else(|| unknown(format!("{checkpoint_id} of {trace_id}")))
    }

    fn record_release(
        &mut self,
        trace_id: TraceId,
        checkpoint_id: CheckpointId,
        failure: Option<&Failure>,
    ) -> Result<(), CatalogError> {
        updated_one(
            checkpoints::record_release(&self.connection, trace_id, checkpoint_id, failure),
            format!("{checkpoint_id} of {trace_id}"),
        )
    }

    fn checkpoint(
        &self,
        trace_id: TraceId,
        checkpoint_id: CheckpointId,
    ) -> Result<Option<CheckpointRecord>, CatalogError> {
        checkpoints::get(&self.connection, trace_id, checkpoint_id).map_err(db)
    }

    fn checkpoints(&self, trace_id: TraceId) -> Result<Vec<CheckpointRecord>, CatalogError> {
        checkpoints::for_trace(&self.connection, trace_id).map_err(db)
    }

    fn operation(
        &self,
        operation_id: OperationId,
    ) -> Result<Option<OperationRecord>, CatalogError> {
        operations::get(&self.connection, operation_id).map_err(db)
    }

    fn operations(&self, trace_id: TraceId) -> Result<Vec<OperationRecord>, CatalogError> {
        operations::for_trace(&self.connection, trace_id).map_err(db)
    }

    fn unfinished_operations(&self) -> Result<Vec<OperationRecord>, CatalogError> {
        operations::unfinished(&self.connection).map_err(db)
    }
}

pub(crate) fn db(error: rusqlite::Error) -> CatalogError {
    CatalogError::new(format!("checkpoint catalog: {error}"))
}

fn unknown(what: impl std::fmt::Display) -> CatalogError {
    CatalogError::new(format!(
        "{what} is not in the checkpoint catalog or already finished"
    ))
}

/// Updates that match no row name an unknown record, or an operation that already finished.
fn updated_one(
    result: rusqlite::Result<usize>,
    what: impl std::fmt::Display,
) -> Result<(), CatalogError> {
    match result.map_err(db)? {
        0 => Err(unknown(what)),
        _ => Ok(()),
    }
}
