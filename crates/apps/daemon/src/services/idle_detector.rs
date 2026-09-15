//! Idle detector scheduler and persistence drain.

use std::collections::BTreeSet;
use std::time::SystemTime;

use control_contract::reply::ControlError;
use idle_contract::{IdleStoreOp, TurnLifecycleEvent, UserInteractionEvent};
use idle_detector::IdleDetector;
use model_core::diagnostics::{DiagnosticKind, DiagnosticRecord, DiagnosticSeverity};
use model_core::ids::TraceId;
use recording_runtime::{RecordingError, SemanticActionBatch};
use storage_core::StorageBackend;

use crate::services::attach::StorageAttachService;
use crate::services::idle_store_error_to_control;

pub(super) struct IdleRuntime {
    pub(super) detector: Option<IdleDetector>,
}

impl IdleRuntime {
    pub(super) fn new(detector: Option<IdleDetector>) -> Self {
        Self { detector }
    }

    pub(super) fn has_pending_ops(&self) -> bool {
        self.detector
            .as_ref()
            .is_some_and(IdleDetector::has_pending_ops)
    }

    pub(super) fn next_deadline(&self) -> Option<SystemTime> {
        self.detector.as_ref().and_then(IdleDetector::next_deadline)
    }

    pub(super) fn prepare_batch(
        &self,
        batch: &mut SemanticActionBatch,
        observed_at: SystemTime,
    ) -> IdleBatchUpdate {
        self.prepare_batch_inner(batch, observed_at, None)
    }

    pub(super) fn prepare_terminal_batch(
        &self,
        batch: &mut SemanticActionBatch,
        observed_at: SystemTime,
        trace_id: TraceId,
    ) -> IdleBatchUpdate {
        self.prepare_batch_inner(batch, observed_at, Some(trace_id))
    }

    fn prepare_batch_inner(
        &self,
        batch: &mut SemanticActionBatch,
        observed_at: SystemTime,
        terminal_trace_id: Option<TraceId>,
    ) -> IdleBatchUpdate {
        let Some(detector) = &self.detector else {
            return IdleBatchUpdate::disabled();
        };
        let links = batch.links().to_vec();
        // Stage changes on a copy so failed persistence cannot advance in-memory state.
        // This clones all detector state per batch, which can be costly for large traces.
        let mut detector = detector.clone();
        let ambiguous_traces = detector.observe_batch(batch.actions_mut(), &links, observed_at);
        if let Some(trace_id) = terminal_trace_id {
            detector.on_trace_ended(trace_id, observed_at);
        }
        let ops = detector.drain_pending_ops();
        IdleBatchUpdate {
            detector: Some(detector),
            ops,
            ambiguous_traces,
        }
    }

    pub(super) fn commit(&mut self, update: IdleBatchUpdate) {
        if let Some(detector) = update.detector {
            self.detector = Some(detector);
        }
    }
}

pub(super) struct IdleBatchUpdate {
    detector: Option<IdleDetector>,
    ops: Vec<IdleStoreOp>,
    ambiguous_traces: BTreeSet<TraceId>,
}

impl IdleBatchUpdate {
    fn disabled() -> Self {
        Self {
            detector: None,
            ops: Vec::new(),
            ambiguous_traces: BTreeSet::new(),
        }
    }

    pub(super) fn persist(&self, storage: &mut dyn StorageBackend) -> Result<(), RecordingError> {
        if self.ops.is_empty() {
            return Ok(());
        }
        storage
            .apply_idle_ops(&self.ops)
            .map_err(|error| RecordingError::new(error.stage, error.message))
    }

    pub(super) fn ambiguous_traces(&self) -> &BTreeSet<TraceId> {
        &self.ambiguous_traces
    }
}

impl StorageAttachService {
    pub(super) fn idle_attribution_diagnostics(
        &mut self,
        trace_ids: &BTreeSet<TraceId>,
        observed_at: SystemTime,
    ) -> Result<Vec<DiagnosticRecord>, ControlError> {
        trace_ids
            .iter()
            .map(|&trace_id| {
                Ok(DiagnosticRecord::new(
                    self.next_diagnostic_id()?,
                    Some(trace_id),
                    DiagnosticKind::RuntimeDropped,
                    DiagnosticSeverity::Warning,
                    observed_at,
                    "semantic action has no unambiguous agent.turn.task_id; idle attribution skipped",
                )
                .with_metadata("component", "idle_detector")
                .with_metadata("code", "idle_action_unattributed"))
            })
            .collect()
    }

    pub(in crate::services) fn report_turn_lifecycle_impl(
        &mut self,
        event: TurnLifecycleEvent,
    ) -> Result<(), ControlError> {
        let Some(detector) = self.idle_runtime.detector.as_mut() else {
            return Ok(());
        };
        event
            .validate()
            .map_err(|message| ControlError::new("invalid_turn_lifecycle", message))?;
        if !detector.on_turn_lifecycle(event) {
            return Err(ControlError::new(
                "idle_task_not_found",
                "terminal turn lifecycle event does not match an active task",
            ));
        }
        self.drain_idle_detector_ops()
    }

    pub(in crate::services) fn report_user_interaction_impl(
        &mut self,
        event: UserInteractionEvent,
    ) -> Result<(), ControlError> {
        let Some(detector) = self.idle_runtime.detector.as_mut() else {
            return Ok(());
        };
        event
            .validate()
            .map_err(|message| ControlError::new("invalid_user_interaction", message))?;
        if !detector.on_user_interaction(event) {
            return Err(ControlError::new(
                "idle_task_not_found",
                "user interaction event does not match an active task",
            ));
        }
        self.drain_idle_detector_ops()
    }

    pub(in crate::services) fn tick_idle_detector_impl(&mut self) -> Result<(), ControlError> {
        let Some(detector) = self.idle_runtime.detector.as_mut() else {
            return Ok(());
        };
        detector.tick(SystemTime::now());
        self.drain_idle_detector_ops()
    }
    // Persist queued idle operations, restoring them if storage fails.
    pub(in crate::services) fn drain_idle_detector_ops(&mut self) -> Result<(), ControlError> {
        let Some(detector) = self.idle_runtime.detector.as_mut() else {
            return Ok(());
        };
        let ops = detector.drain_pending_ops();
        if ops.is_empty() {
            return Ok(());
        }
        if let Err(error) = self.storage.apply_idle_ops(&ops) {
            self.idle_runtime
                .detector
                .as_mut()
                .expect("idle detector is enabled while draining its operations")
                .restore_pending_ops(ops);
            return Err(idle_store_error_to_control(error));
        }
        Ok(())
    }
}
