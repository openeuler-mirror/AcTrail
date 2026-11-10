//! Commands: requests to change a context's checkpoints. Each checks the context's state and
//! records the request under the catalog lock, queues the work and returns; it is refused when the
//! state does not allow it.

use std::time::SystemTime;

use checkpoint::TraceProcess;

use super::scheduler::{Job, TraceState};
use super::{CheckpointCoordinator, CoordinatorError, unknown_checkpoint, unknown_trace};
use crate::model::{
    CheckpointId, NewOperation, OperationId, OperationKind, OperationRecord, Phase, ScopeConfig,
    SnapshotContext, TraceOwner, TraceRecord, Trigger,
};
use crate::ports::CheckpointCatalog;
use crate::scope::plan_scopes;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LaunchRequest {
    pub context: SnapshotContext,
    pub owner: TraceOwner,
    /// The Trace root, still stopped at the launch barrier.
    pub trace_process: TraceProcess,
    pub scopes: Vec<ScopeConfig>,
    pub requested_at: SystemTime,
}

impl CheckpointCoordinator {
    /// Connects the Trace's backend and takes the initial checkpoint; the result arrives as a
    /// `CompletedJob::Launch` completion.
    pub fn launch(&self, request: LaunchRequest) -> Result<(), CoordinatorError> {
        for group in plan_scopes(&request.scopes)? {
            if !self.shared.factory.takes_scopes(&group.backend_instance) {
                return Err(CoordinatorError::new(
                    "unknown_backend",
                    format!(
                        "backend instance {} is not loaded or takes no scopes",
                        group.backend_instance
                    ),
                ));
            }
        }
        let context = request.context.clone();
        self.shared.enqueue(&context, Job::Launch(request));
        Ok(())
    }

    /// Queues a save of every scope of a running context.
    pub fn save(
        &self,
        context: &SnapshotContext,
        trigger: Trigger,
    ) -> Result<OperationId, CoordinatorError> {
        self.shared.persist_pending(context)?;
        let trace_id = context.trace_id;
        let mut catalog = self.shared.catalog();
        let trace = catalog
            .trace(trace_id)?
            .ok_or_else(|| unknown_trace(trace_id))?;
        {
            let mut traces = self.shared.traces();
            require_running(&trace, traces.entry(context.clone()).or_default())?;
        }
        let operation = catalog.create_operation(&NewOperation {
            trace_id,
            kind: OperationKind::Save,
            trigger,
            phase: Phase::Queued,
            requested_at: SystemTime::now(),
            target_checkpoint: None,
        })?;
        drop(catalog);
        self.shared.enqueue(context, Job::Save { operation });
        Ok(operation)
    }

    /// Queues a restore to one of the context's checkpoints. It waits for the next LLM request
    /// boundary, so the Agent is stopped while it waits for a response, unless a restore that did
    /// not succeed already holds it. `force` starts a waiting restore at once, and a Trace that
    /// ends while a restore waits has it run directly.
    pub fn restore(
        &self,
        context: &SnapshotContext,
        target: CheckpointId,
    ) -> Result<OperationId, CoordinatorError> {
        self.shared.persist_pending(context)?;
        let trace_id = context.trace_id;
        let mut catalog = self.shared.catalog();
        let trace = catalog
            .trace(trace_id)?
            .ok_or_else(|| unknown_trace(trace_id))?;
        let checkpoint = catalog
            .checkpoint(trace_id, target)?
            .ok_or_else(|| unknown_checkpoint(target))?;
        let mut traces = self.shared.traces();
        let state = traces.entry(context.clone()).or_default();
        require_running(&trace, state)?;
        if !checkpoint.restorable() {
            return Err(CoordinatorError::new(
                "not_restorable",
                format!("{target} was invalidated or deleted"),
            ));
        }
        if let Some(running) = unfinished_restore(&**catalog, trace_id)? {
            return Err(restore_in_progress(&trace, &running));
        }
        let direct = state.held.is_some();
        let operation = catalog.create_operation(&NewOperation {
            trace_id,
            kind: OperationKind::Restore,
            trigger: Trigger::Manual,
            phase: if direct {
                Phase::Queued
            } else {
                Phase::WaitingBoundary
            },
            requested_at: SystemTime::now(),
            target_checkpoint: Some(target),
        })?;
        drop(catalog);
        if !direct {
            state.waiting_restore = Some(operation);
            return Ok(operation);
        }
        drop(traces);
        self.shared.enqueue(
            context,
            Job::Restore {
                operation,
                boundary: None,
            },
        );
        Ok(operation)
    }

    /// Starts a waiting restore now and returns its context. Calls for a restore already
    /// triggered or finished are accepted without queuing another restore.
    pub fn force(&self, operation: OperationId) -> Result<SnapshotContext, CoordinatorError> {
        let catalog = self.shared.catalog();
        let restore = catalog
            .operation(operation)?
            .filter(|record| record.kind == OperationKind::Restore)
            .ok_or_else(|| {
                CoordinatorError::new(
                    "not_waiting",
                    format!("{operation} is not a restore operation"),
                )
            })?;
        let context = SnapshotContext {
            trace_id: restore.trace_id,
        };
        let mut traces = self.shared.traces();
        if let Some(state) = traces.get_mut(&context)
            && state.waiting_restore == Some(operation)
        {
            state.waiting_restore = None;
            drop(traces);
            drop(catalog);
            self.shared.enqueue(
                &context,
                Job::Restore {
                    operation,
                    boundary: None,
                },
            );
        }
        Ok(context)
    }

    /// Marks the checkpoint deleted, then queues the release of its snapshot.
    pub fn delete(
        &self,
        context: &SnapshotContext,
        checkpoint_id: CheckpointId,
    ) -> Result<(), CoordinatorError> {
        self.shared.persist_pending(context)?;
        let trace_id = context.trace_id;
        let mut catalog = self.shared.catalog();
        let trace = catalog
            .trace(trace_id)?
            .ok_or_else(|| unknown_trace(trace_id))?;
        catalog
            .checkpoint(trace_id, checkpoint_id)?
            .ok_or_else(|| unknown_checkpoint(checkpoint_id))?;
        {
            let mut traces = self.shared.traces();
            require_connected(&trace, traces.entry(context.clone()).or_default())?;
        }
        let targeted = catalog.operations(trace_id)?.into_iter().any(|operation| {
            operation.phase != Phase::Finished && operation.target_checkpoint == Some(checkpoint_id)
        });
        if targeted {
            return Err(CoordinatorError::new(
                "checkpoint_in_use",
                format!("an unfinished restore targets {checkpoint_id}"),
            ));
        }
        let snapshot_id =
            catalog.mark_checkpoint_deleted(trace_id, checkpoint_id, SystemTime::now())?;
        drop(catalog);
        self.shared.enqueue(
            context,
            Job::Release {
                checkpoint_id,
                snapshot_id,
            },
        );
        Ok(())
    }

    /// Gives up the context's checkpoints before its Trace ends. Once the work queued before it
    /// is done, the remaining checkpoints are deleted, the backend disconnects, and what an
    /// unsuccessful restore holds goes on. From now on the context takes no saves or restores, and
    /// its LLM requests go on at once. When the last disconnect failed, this retries it, also
    /// after the Trace ended.
    pub fn disconnect(&self, context: &SnapshotContext) -> Result<(), CoordinatorError> {
        self.shared.persist_pending(context)?;
        let trace_id = context.trace_id;
        let catalog = self.shared.catalog();
        let trace = catalog
            .trace(trace_id)?
            .ok_or_else(|| unknown_trace(trace_id))?;
        let mut traces = self.shared.traces();
        let state = traces.entry(context.clone()).or_default();
        if !std::mem::take(&mut state.disconnect_failed) {
            require_connected(&trace, state)?;
            if let Some(running) = unfinished_restore(&**catalog, trace_id)? {
                return Err(restore_in_progress(&trace, &running));
            }
            state.disconnecting = true;
        }
        drop(traces);
        drop(catalog);
        self.shared
            .enqueue(context, Job::Disconnect { ended_at: None });
        Ok(())
    }

    /// Continues the processes a failed restore left stopped, after any queued work.
    pub fn resume(&self, context: &SnapshotContext) -> Result<(), CoordinatorError> {
        self.shared
            .catalog()
            .trace(context.trace_id)?
            .ok_or_else(|| unknown_trace(context.trace_id))?;
        self.shared.enqueue(context, Job::Resume);
        Ok(())
    }
}

/// Saves and restores need a Trace that has not ended.
fn require_running(trace: &TraceRecord, state: &TraceState) -> Result<(), CoordinatorError> {
    if trace.ended_at.is_some() {
        return Err(CoordinatorError::new(
            "trace_ended",
            format!(
                "{} has ended, so it can be neither saved nor restored",
                trace.trace_id
            ),
        ));
    }
    require_connected(trace, state)
}

fn require_connected(trace: &TraceRecord, state: &TraceState) -> Result<(), CoordinatorError> {
    if trace.disconnected.is_some() || state.disconnecting {
        return Err(CoordinatorError::new(
            "disconnected",
            format!("{} is disconnected", trace.trace_id),
        ));
    }
    if !state.initialized {
        return Err(CoordinatorError::new(
            "launch_incomplete",
            format!(
                "{} has not completed its initial checkpoint",
                trace.trace_id
            ),
        ));
    }
    Ok(())
}

fn unfinished_restore(
    catalog: &dyn CheckpointCatalog,
    trace_id: model_core::ids::TraceId,
) -> Result<Option<OperationRecord>, CoordinatorError> {
    Ok(catalog.operations(trace_id)?.into_iter().find(|operation| {
        operation.kind == OperationKind::Restore && operation.phase != Phase::Finished
    }))
}

fn restore_in_progress(trace: &TraceRecord, running: &OperationRecord) -> CoordinatorError {
    CoordinatorError::new(
        "restore_in_progress",
        format!(
            "{} of {} has not finished",
            running.operation_id, trace.trace_id
        ),
    )
}
