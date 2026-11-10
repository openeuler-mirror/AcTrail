//! Accepts checkpoint work and runs it in order per Trace.
//!
//! The public methods come in three groups, one file each. Commands change a Trace's checkpoints
//! on request and may be refused; queries read them; events report what the daemon observed. A
//! command or event does its catalog checks and writes under the catalog lock and returns at once;
//! backend calls and process control run as jobs on a worker thread per Trace. Queries read
//! through a connection of their own, so they wait for neither.

mod commands;
mod events;
mod jobs;
mod queries;
mod recover;
mod scheduler;

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::SystemTime;

use abi_stable::std_types::RResult;
use checkpoint::CheckpointBackend;
use model_core::ids::TraceId;

pub use self::commands::LaunchRequest;
pub use self::events::BoundaryAction;
use self::scheduler::TraceState;
use crate::factory::CheckpointFactory;
use crate::model::{CheckpointId, OperationId, SnapshotContext};
use crate::ports::{CatalogError, CatalogReader, CheckpointCatalog, ProcessControl};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CoordinatorError {
    pub code: String,
    pub message: String,
}

impl CoordinatorError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for CoordinatorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for CoordinatorError {}

impl From<CatalogError> for CoordinatorError {
    fn from(error: CatalogError) -> Self {
        Self::new("catalog", error.message)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompletedJob {
    /// Bind and the initial save; the daemon answers `TrackAdd` with this result.
    Launch,
    Operation(OperationId),
    /// An automatic save decision, including one that skipped an unchanged Trace.
    Boundary,
    Release(CheckpointId),
    /// Disconnecting the backend after `disconnect` or the end of the Trace.
    Disconnect,
    Resume,
}

/// The caller's handle on an Agent waiting at an LLM request boundary. The coordinator returns
/// it when the Agent may go on.
pub type Hold = u64;

/// Reported after each job. Backend failures are part of the recorded operation; `result` is an
/// error only when the job itself could not finish, or when a launch produced no checkpoint.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Completion {
    pub context: SnapshotContext,
    pub job: CompletedJob,
    pub result: Result<(), CoordinatorError>,
    /// Agents that may go on past their LLM request boundary.
    pub released: Vec<Hold>,
}

pub trait CompletionSink: Send + Sync {
    fn completed(&self, completion: Completion);
}

pub struct CheckpointCoordinator {
    shared: Arc<Shared>,
    /// Only queries use it, so they never wait for the catalog lock.
    reader: Box<dyn CatalogReader + Send>,
}

pub(crate) struct Shared {
    catalog: Mutex<Box<dyn CheckpointCatalog + Send>>,
    factory: CheckpointFactory,
    processes: Box<dyn ProcessControl>,
    sink: Box<dyn CompletionSink>,
    traces: Mutex<BTreeMap<SnapshotContext, TraceState>>,
}

impl CheckpointCoordinator {
    pub fn new(
        catalog: Box<dyn CheckpointCatalog + Send>,
        reader: Box<dyn CatalogReader + Send>,
        factory: CheckpointFactory,
        processes: Box<dyn ProcessControl>,
        sink: Box<dyn CompletionSink>,
    ) -> Self {
        Self {
            shared: Arc::new(Shared {
                catalog: Mutex::new(catalog),
                factory,
                processes,
                sink,
                traces: Mutex::new(BTreeMap::new()),
            }),
            reader,
        }
    }

    /// Closes what an earlier daemon left behind. Call once at startup, before submitting work.
    /// Returns how many operations it finished.
    pub fn recover(&self) -> Result<usize, CoordinatorError> {
        recover::close_earlier_daemon(&mut **self.shared.catalog(), SystemTime::now())
    }
}

impl Shared {
    pub(crate) fn catalog(&self) -> MutexGuard<'_, Box<dyn CheckpointCatalog + Send>> {
        lock(&self.catalog)
    }

    /// Calls the context's backend without holding a lock, so calls of different contexts run at
    /// the same time. Only the context's worker calls it, so calls never overlap.
    pub(crate) fn call<R, E>(
        &self,
        context: &SnapshotContext,
        call: impl FnOnce(&mut CheckpointBackend) -> RResult<R, E>,
    ) -> Result<R, E> {
        let mut backend = self.take_backend(context);
        let result = call(&mut backend).into_result();
        self.set_backend(context, backend);
        result
    }

    pub(crate) fn processes(&self) -> &dyn ProcessControl {
        &*self.processes
    }
}

fn lock<T: ?Sized>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn unknown_trace(trace_id: TraceId) -> CoordinatorError {
    CoordinatorError::new(
        "unknown_trace",
        format!("{trace_id} has no checkpoint configuration"),
    )
}

fn unknown_checkpoint(checkpoint_id: CheckpointId) -> CoordinatorError {
    CoordinatorError::new(
        "unknown_checkpoint",
        format!("{checkpoint_id} is not a checkpoint of this Trace"),
    )
}
