use model_core::diagnostics::DiagnosticRecord;
use model_core::event::DomainEvent;
use model_core::ids::TraceId;
use model_core::payload::PayloadSegment;
use model_core::process::ProcessMembership;
use model_core::trace::TraceRecord;
use storage_core::StorageBackend;

use crate::semantic::{RecordingError, SemanticActionBatch, SemanticActionRecorder};

#[derive(Default)]
pub(crate) struct ObservedRecordBatch {
    events: Vec<DomainEvent>,
    payload_segments: Vec<PayloadSegment>,
    diagnostics: Vec<DiagnosticRecord>,
    semantic_actions: SemanticActionBatch,
    trace_states: Vec<TraceStateRecord>,
}

impl ObservedRecordBatch {
    pub(crate) fn from_live_events(
        events: Vec<DomainEvent>,
        diagnostics: Vec<DiagnosticRecord>,
        semantic_actions: SemanticActionBatch,
        trace_states: Vec<TraceStateRecord>,
    ) -> Self {
        Self {
            events,
            payload_segments: Vec::new(),
            diagnostics,
            semantic_actions,
            trace_states,
        }
    }

    pub(crate) fn from_payload_segment(
        segment: PayloadSegment,
        semantic_actions: SemanticActionBatch,
    ) -> Self {
        // Payload retention depends on each segment write being visible to the next check.
        Self {
            events: Vec::new(),
            payload_segments: vec![segment],
            diagnostics: Vec::new(),
            semantic_actions,
            trace_states: Vec::new(),
        }
    }

    pub(crate) fn from_event(event: DomainEvent, semantic_actions: SemanticActionBatch) -> Self {
        Self {
            events: vec![event],
            payload_segments: Vec::new(),
            diagnostics: Vec::new(),
            semantic_actions,
            trace_states: Vec::new(),
        }
    }

    pub(crate) fn from_semantic_actions(semantic_actions: SemanticActionBatch) -> Self {
        Self {
            events: Vec::new(),
            payload_segments: Vec::new(),
            diagnostics: Vec::new(),
            semantic_actions,
            trace_states: Vec::new(),
        }
    }

    pub(crate) fn from_trace_state(trace_state: TraceStateRecord) -> Self {
        Self {
            events: Vec::new(),
            payload_segments: Vec::new(),
            diagnostics: Vec::new(),
            semantic_actions: SemanticActionBatch::default(),
            trace_states: vec![trace_state],
        }
    }

    pub(crate) fn from_diagnostic(diagnostic: DiagnosticRecord) -> Self {
        Self {
            events: Vec::new(),
            payload_segments: Vec::new(),
            diagnostics: vec![diagnostic],
            semantic_actions: SemanticActionBatch::default(),
            trace_states: Vec::new(),
        }
    }
}

pub struct TraceStateRecord {
    trace: TraceRecord,
    memberships: Vec<ProcessMembership>,
}

impl TraceStateRecord {
    pub fn new(trace: TraceRecord, memberships: Vec<ProcessMembership>) -> Self {
        Self { trace, memberships }
    }
}

pub(crate) struct ObservedRecordCommit {
    semantic_actions: SemanticActionBatch,
}

impl ObservedRecordCommit {
    pub(crate) fn into_semantic_actions(self) -> SemanticActionBatch {
        self.semantic_actions
    }
}

pub(crate) struct ObservedRecordRecorder<'a> {
    storage: &'a mut dyn StorageBackend,
}

impl<'a> ObservedRecordRecorder<'a> {
    pub(crate) fn new(storage: &'a mut dyn StorageBackend) -> Self {
        Self { storage }
    }

    pub(crate) fn persist_batch(
        &mut self,
        batch: ObservedRecordBatch,
    ) -> Result<ObservedRecordCommit, RecordingError> {
        let ObservedRecordBatch {
            events,
            payload_segments,
            diagnostics,
            semantic_actions,
            trace_states,
        } = batch;

        for event in events {
            self.storage.append_event(event)?;
        }
        for segment in payload_segments {
            self.storage.append_payload_segment(segment)?;
        }
        {
            let mut recorder = SemanticActionRecorder::new(&mut *self.storage);
            recorder.persist_batch(semantic_actions.as_record_batch())?;
        }
        for diagnostic in diagnostics {
            self.storage.append_diagnostic(diagnostic)?;
        }
        // Persist trace snapshots after their observed records in the same transaction.
        for state in trace_states {
            self.storage.create_trace(state.trace)?;
            for membership in state.memberships {
                self.storage.upsert_membership(membership)?;
            }
        }

        Ok(ObservedRecordCommit { semantic_actions })
    }
}

pub struct ObservedRecordWriteSession<'a> {
    storage: &'a mut dyn StorageBackend,
}

impl<'a> ObservedRecordWriteSession<'a> {
    pub(crate) fn new(storage: &'a mut dyn StorageBackend) -> Self {
        Self { storage }
    }

    pub fn retained_payload_bytes(&self, trace_id: TraceId) -> Result<u64, RecordingError> {
        self.storage
            .retained_payload_bytes(trace_id)
            .map_err(RecordingError::from)
    }

    pub(crate) fn persist_batch(
        &mut self,
        batch: ObservedRecordBatch,
    ) -> Result<ObservedRecordCommit, RecordingError> {
        ObservedRecordRecorder::new(self.storage).persist_batch(batch)
    }

    pub fn persist_payload_segment(
        &mut self,
        segment: PayloadSegment,
        semantic_actions: SemanticActionBatch,
    ) -> Result<SemanticActionBatch, RecordingError> {
        self.persist_batch(ObservedRecordBatch::from_payload_segment(
            segment,
            semantic_actions,
        ))
        .map(ObservedRecordCommit::into_semantic_actions)
    }

    pub fn persist_semantic_actions(
        &mut self,
        semantic_actions: SemanticActionBatch,
    ) -> Result<SemanticActionBatch, RecordingError> {
        self.persist_batch(ObservedRecordBatch::from_semantic_actions(semantic_actions))
            .map(ObservedRecordCommit::into_semantic_actions)
    }

    pub fn persist_event(
        &mut self,
        event: DomainEvent,
        semantic_actions: SemanticActionBatch,
    ) -> Result<SemanticActionBatch, RecordingError> {
        self.persist_batch(ObservedRecordBatch::from_event(event, semantic_actions))
            .map(ObservedRecordCommit::into_semantic_actions)
    }
}
