//! Checkpoint control commands, their authorization and the views they return.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use checkpoint_core::model::{
    CheckpointId, CheckpointRecord, Failure, FilterMode, OperationId, OperationRecord, Release,
    SnapshotContext, Trigger,
};
use control_contract::checkpoint::{
    CheckpointAction, CheckpointFilterMode, CheckpointReply, CheckpointView, OperationView,
};
use control_contract::reply::ControlError;
use model_core::ids::TraceId;
use trace_runtime::TraceOwnerPrincipal;

use super::service::{CheckpointService, coordinator_error};

impl CheckpointService {
    /// The Trace a command acts on, used to authorize the caller.
    pub(crate) fn command_trace(&self, action: &CheckpointAction) -> Result<TraceId, ControlError> {
        match *action {
            CheckpointAction::Save { trace_id }
            | CheckpointAction::Restore { trace_id, .. }
            | CheckpointAction::Delete { trace_id, .. }
            | CheckpointAction::Disconnect { trace_id }
            | CheckpointAction::Resume { trace_id }
            | CheckpointAction::List { trace_id, .. }
            | CheckpointAction::Operations { trace_id }
            | CheckpointAction::Status { trace_id } => Ok(trace_id),
            CheckpointAction::Operation { operation_id }
            | CheckpointAction::Force { operation_id } => self
                .coordinator
                .operation(OperationId::new(operation_id))
                .map(|operation| operation.trace_id)
                .map_err(coordinator_error),
        }
    }

    /// The owner recorded at launch, which stays available after the Trace leaves the runtime.
    pub(crate) fn trace_owner(
        &self,
        trace_id: TraceId,
    ) -> Result<TraceOwnerPrincipal, ControlError> {
        let owner = self
            .coordinator
            .owner(&SnapshotContext { trace_id })
            .map_err(coordinator_error)?;
        Ok(TraceOwnerPrincipal {
            uid: owner.uid,
            pid_namespace: owner.pid_namespace,
            mount_namespace: owner.mount_namespace,
            host_pid_namespace: owner.host_pid_namespace,
            host_mount_namespace: owner.host_mount_namespace,
        })
    }

    pub(crate) fn handle(&self, action: CheckpointAction) -> Result<CheckpointReply, ControlError> {
        let accepted = |operation: OperationId| CheckpointReply::OperationAccepted {
            operation_id: operation.get(),
        };
        let coordinator = &self.coordinator;
        let context = |trace_id| SnapshotContext { trace_id };
        match action {
            CheckpointAction::Status { trace_id } => {
                let status = coordinator.status(&context(trace_id));
                Ok(CheckpointReply::Status {
                    held: status.held,
                    busy: status.busy,
                })
            }
            CheckpointAction::Save { trace_id } => coordinator
                .save(&context(trace_id), Trigger::Manual)
                .map(accepted),
            CheckpointAction::Restore {
                trace_id,
                checkpoint_id,
            } => coordinator
                .restore(&context(trace_id), CheckpointId::new(checkpoint_id))
                .map(accepted),
            CheckpointAction::Delete {
                trace_id,
                checkpoint_id,
            } => coordinator
                .delete(&context(trace_id), CheckpointId::new(checkpoint_id))
                .map(|()| CheckpointReply::Accepted),
            CheckpointAction::Disconnect { trace_id } => coordinator
                .disconnect(&context(trace_id))
                .map(|()| CheckpointReply::Accepted),
            CheckpointAction::Force { operation_id } => coordinator
                .force(OperationId::new(operation_id))
                .map(|_| CheckpointReply::Accepted),
            CheckpointAction::Resume { trace_id } => coordinator
                .resume(&context(trace_id))
                .map(|()| CheckpointReply::Accepted),
            CheckpointAction::List {
                trace_id,
                filter_mode,
            } => coordinator
                .list(&context(trace_id), &filter(filter_mode)?)
                .map(|checkpoints| {
                    CheckpointReply::Checkpoints(checkpoints.iter().map(checkpoint_view).collect())
                }),
            CheckpointAction::Operations { trace_id } => coordinator
                .operations(&context(trace_id))
                .map(|operations| {
                    CheckpointReply::Operations(operations.iter().map(operation_view).collect())
                }),
            CheckpointAction::Operation { operation_id } => coordinator
                .operation(OperationId::new(operation_id))
                .map(|operation| CheckpointReply::Operation(Box::new(operation_view(&operation)))),
        }
        .map_err(coordinator_error)
    }
}

fn filter(filter_mode: CheckpointFilterMode) -> Result<FilterMode, ControlError> {
    let time = |nanos: u64| {
        if nanos > i64::MAX as u64 {
            return Err(ControlError::new(
                "invalid_filter",
                "checkpoint filter times must fit in signed 64-bit Unix nanoseconds",
            ));
        }
        Ok(UNIX_EPOCH + Duration::from_nanos(nanos))
    };
    Ok(match filter_mode {
        CheckpointFilterMode::All => FilterMode::All,
        CheckpointFilterMode::Last(count) => FilterMode::Last(count),
        CheckpointFilterMode::Created {
            since_unix_nanos,
            until_unix_nanos,
        } => FilterMode::Created {
            since: since_unix_nanos.map(time).transpose()?,
            until: until_unix_nanos.map(time).transpose()?,
        },
    })
}

fn checkpoint_view(checkpoint: &CheckpointRecord) -> CheckpointView {
    let (release, release_error) = match checkpoint
        .deletion
        .as_ref()
        .map(|deletion| &deletion.release)
    {
        None => (None, None),
        Some(Release::Pending) => (Some("pending"), None),
        Some(Release::Released) => (Some("released"), None),
        Some(Release::Failed(failure)) => (Some("failed"), Some(failure_text(failure))),
    };
    CheckpointView {
        checkpoint_id: checkpoint.checkpoint_id.get(),
        operation_id: checkpoint.operation_id.get(),
        created_at_unix_nanos: unix_nanos(checkpoint.created_at),
        restorable: checkpoint.restorable(),
        invalidated_by: checkpoint.invalidated_by.map(OperationId::get),
        deleted_at_unix_nanos: checkpoint
            .deletion
            .as_ref()
            .map(|deletion| unix_nanos(deletion.deleted_at)),
        release: release.map(str::to_string),
        release_error,
    }
}

fn operation_view(operation: &OperationRecord) -> OperationView {
    OperationView {
        operation_id: operation.operation_id.get(),
        trace_id: operation.trace_id.get(),
        kind: operation.kind.as_str().to_string(),
        trigger: operation.trigger.as_str().to_string(),
        phase: operation.phase.as_str().to_string(),
        outcome: operation
            .outcome
            .map(|outcome| outcome.as_str().to_string()),
        requested_at_unix_nanos: unix_nanos(operation.requested_at),
        started_at_unix_nanos: operation.started_at.map(unix_nanos),
        finished_at_unix_nanos: operation.finished_at.map(unix_nanos),
        target_checkpoint: operation.target_checkpoint.map(CheckpointId::get),
        created_checkpoint: operation.created_checkpoint.map(CheckpointId::get),
        error: operation.failure.as_ref().map(failure_text),
        invalidated_checkpoints: operation
            .invalidated_checkpoints
            .iter()
            .map(|checkpoint| checkpoint.get())
            .collect(),
    }
}

fn failure_text(failure: &Failure) -> String {
    format!("{}: {}", failure.code, failure.message)
}

fn unix_nanos(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map(|duration| u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX))
        .unwrap_or_default()
}
