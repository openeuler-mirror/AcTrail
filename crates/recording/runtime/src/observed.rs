use model_core::diagnostics::DiagnosticRecord;
use model_core::event::DomainEvent;
use model_core::ids::TraceId;
use model_core::payload::PayloadSegment;
use model_core::process::{ProcessMembership, ProcessRecord};
use model_core::trace::TraceRecord;
use storage_core::StorageBackend;

use crate::semantic::{
    RecordingError, SemanticActionBatch, SemanticActionPersistenceAccumulator,
    SemanticActionRecorder,
};

#[derive(Default)]
pub(crate) struct ObservedRecordBatch {
    events: Vec<DomainEvent>,
    payload_segments: Vec<PayloadSegment>,
    diagnostics: Vec<DiagnosticRecord>,
    semantic_actions: SemanticActionBatch,
    trace_states: Vec<TraceStateRecord>,
    process_records: Vec<ProcessRecord>,
}

impl ObservedRecordBatch {
    pub(crate) fn semantic_actions(&self) -> &SemanticActionBatch {
        &self.semantic_actions
    }

    pub(crate) fn from_live_events(
        events: Vec<DomainEvent>,
        diagnostics: Vec<DiagnosticRecord>,
        semantic_actions: SemanticActionBatch,
        trace_states: Vec<TraceStateRecord>,
        process_records: Vec<ProcessRecord>,
    ) -> Self {
        Self {
            events,
            payload_segments: Vec::new(),
            diagnostics,
            semantic_actions,
            trace_states,
            process_records,
        }
    }

    pub(crate) fn from_semantic_actions(semantic_actions: SemanticActionBatch) -> Self {
        Self {
            events: Vec::new(),
            payload_segments: Vec::new(),
            diagnostics: Vec::new(),
            semantic_actions,
            trace_states: Vec::new(),
            process_records: Vec::new(),
        }
    }

    pub(crate) fn from_trace_state(
        trace_state: TraceStateRecord,
        process_records: Vec<ProcessRecord>,
    ) -> Self {
        Self {
            events: Vec::new(),
            payload_segments: Vec::new(),
            diagnostics: Vec::new(),
            semantic_actions: SemanticActionBatch::default(),
            trace_states: vec![trace_state],
            process_records,
        }
    }

    pub(crate) fn from_diagnostic(diagnostic: DiagnosticRecord) -> Self {
        Self {
            events: Vec::new(),
            payload_segments: Vec::new(),
            diagnostics: vec![diagnostic],
            semantic_actions: SemanticActionBatch::default(),
            trace_states: Vec::new(),
            process_records: Vec::new(),
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
            process_records,
        } = batch;

        for record in process_records {
            self.storage.upsert_process_record(record)?;
        }
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
        for segment in semantic_actions.payload_segments() {
            self.storage.append_payload_segment(segment.clone())?;
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
    semantic_actions: SemanticActionPersistenceAccumulator,
}

impl<'a> ObservedRecordWriteSession<'a> {
    pub(crate) fn new(storage: &'a mut dyn StorageBackend) -> Self {
        Self {
            storage,
            semantic_actions: SemanticActionPersistenceAccumulator::default(),
        }
    }

    pub fn retained_payload_bytes(&self, trace_id: TraceId) -> Result<u64, RecordingError> {
        self.storage
            .retained_payload_bytes(trace_id)
            .map_err(RecordingError::from)
    }

    pub fn persist_process_record(&mut self, record: ProcessRecord) -> Result<(), RecordingError> {
        self.storage
            .upsert_process_record(record)
            .map_err(RecordingError::from)
    }

    pub fn persist_trace_state(
        &mut self,
        trace_state: TraceStateRecord,
    ) -> Result<(), RecordingError> {
        self.storage.create_trace(trace_state.trace)?;
        for membership in trace_state.memberships {
            self.storage.upsert_membership(membership)?;
        }
        Ok(())
    }

    pub fn persist_payload_segment(
        &mut self,
        segment: PayloadSegment,
        semantic_actions: SemanticActionBatch,
    ) -> Result<(), RecordingError> {
        self.storage.append_payload_segment(segment)?;
        self.semantic_actions.push_batch(semantic_actions)
    }

    pub fn persist_semantic_actions(
        &mut self,
        semantic_actions: SemanticActionBatch,
    ) -> Result<(), RecordingError> {
        self.semantic_actions.push_batch(semantic_actions)
    }

    pub fn persist_event(
        &mut self,
        event: DomainEvent,
        semantic_actions: SemanticActionBatch,
    ) -> Result<(), RecordingError> {
        self.storage.append_event(event)?;
        self.semantic_actions.push_batch(semantic_actions)
    }

    pub(crate) fn finish(self) -> Result<(), RecordingError> {
        self.semantic_actions.persist(self.storage)
    }
}
