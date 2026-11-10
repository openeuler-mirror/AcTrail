//! Launch: connect the Trace's backend, record the Trace and take the initial checkpoint.

use checkpoint::{SnapshotContext as BackendContext, SnapshotScope};

use super::disconnect;
use super::save;
use crate::coordinator::{CoordinatorError, LaunchRequest, Shared};
use crate::model::{NewOperation, OperationKind, Phase, RootProcess, TraceRecord, Trigger};
use crate::scope::plan_scopes;

/// Succeeds only when the initial checkpoint exists, since the launch fails otherwise.
pub(crate) fn run(shared: &Shared, request: LaunchRequest) -> Result<(), CoordinatorError> {
    let context = request.context.clone();
    let groups = plan_scopes(&request.scopes)?;
    let scopes: Vec<_> = groups
        .iter()
        .flat_map(|group| group.scopes.iter().cloned())
        .collect();
    let mut backend = shared.factory.create(&groups)?;
    backend
        .connect(&BackendContext {
            trace_id: context.trace_id.to_string().into(),
            trace_process: request.trace_process.clone(),
            scopes: scopes
                .iter()
                .map(|scope| SnapshotScope {
                    id: scope.id.as_str().into(),
                    root: scope.root.as_str().into(),
                    excludes: scope
                        .excludes
                        .iter()
                        .map(|path| path.as_str().into())
                        .collect(),
                })
                .collect(),
        })
        .into_result()
        .map_err(|error| CoordinatorError::new("connect_failed", error.message.into_string()))?;
    shared.set_backend(&context, backend);

    let result = initial_checkpoint(shared, request);
    if result.is_err() {
        // The Agent never starts, so the backend has no further use. The launch reports the
        // first error; the catalog that caused it may not take this record either.
        drop(disconnect::record(shared, &context));
    }
    result
}

fn initial_checkpoint(shared: &Shared, request: LaunchRequest) -> Result<(), CoordinatorError> {
    let context = &request.context;
    let trace = TraceRecord {
        trace_id: context.trace_id,
        owner: request.owner,
        root: RootProcess {
            host_pid: request.trace_process.host_pid,
            start_time_ticks: request.trace_process.start_time_ticks,
        },
        scopes: request.scopes,
        created_at: request.requested_at,
        ended_at: None,
        disconnected: None,
    };
    let operation = {
        let mut catalog = shared.catalog();
        catalog.create_trace(&trace)?;
        catalog.create_operation(&NewOperation {
            trace_id: context.trace_id,
            kind: OperationKind::Save,
            trigger: Trigger::Launch,
            phase: Phase::Queued,
            requested_at: request.requested_at,
            target_checkpoint: None,
        })?
    };
    match save::run(shared, context, operation, None)? {
        Some(_) => Ok(()),
        None => Err(CoordinatorError::new(
            "save_failed",
            format!("the initial save failed; {operation} has the details"),
        )),
    }
}
