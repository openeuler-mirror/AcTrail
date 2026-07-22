//! Batched live-event persistence for deferred seccomp observations.

use std::collections::{BTreeMap, BTreeSet};

use collector_event::{RawCollectorEvent, RawObservationPayload};
use config_core::daemon::FileMetadataRetention;
use control_contract::reply::ControlError;
use ingest_runtime::{AllowPolicy, IngestPipeline};
use model_core::diagnostics::{DiagnosticKind, DiagnosticRecord};
use model_core::event::{DomainEvent, EventPayload};
use model_core::ids::TraceId;
use model_core::process::{ProcessIdentity, ProcessRecord};
use recording_runtime::{SemanticActionBatch, TraceStateRecord};
use trace_runtime::registry::TraceRuntime;

use crate::services::attach::StorageAttachService;
use crate::services::identity::RuntimeProcessEventApplier;

impl StorageAttachService {
    pub(in crate::services) fn process_live_event_batch(
        &mut self,
        trace_runtime: &mut TraceRuntime,
        raw_events: Vec<RawCollectorEvent>,
    ) -> Result<(), ControlError> {
        if raw_events.is_empty() {
            return Ok(());
        }
        let input_events = raw_events.len();
        let mut retained_events = 0usize;
        let mut semantic_action_count = 0usize;
        let mut semantic_link_count = 0usize;
        let mut batch = LiveEventBatch::default();
        for raw_event in raw_events {
            let observed_at = raw_event.envelope.observed_at;
            let parent_observation = match &raw_event.payload {
                RawObservationPayload::Process { parent, .. } => parent.clone(),
                _ => None,
            };
            let (process, process_record) =
                self.resolve_process_observation(raw_event.envelope.process.clone())?;
            if let Some(record) = process_record {
                batch.process_records.insert(record.identity, record);
            }
            let parent = parent_observation
                .map(|observation| self.resolve_process_observation(observation))
                .transpose()?;
            if let Some((_, Some(record))) = &parent {
                batch
                    .process_records
                    .insert(record.identity, record.clone());
            }
            let parent = parent.map(|(identity, _)| identity);
            let matched =
                RuntimeProcessEventApplier::new(trace_runtime, &mut self.process_registry)
                    .apply(&raw_event, process, parent)?;
            let matched_trace_id = matched.as_ref().map(|matched| matched.trace_id);
            let event_id = self.next_event_id()?;
            let label_event_id = if self.provider_classification_enabled
                && matches!(&raw_event.payload, RawObservationPayload::Net { .. })
            {
                Some(self.next_event_id()?)
            } else {
                None
            };
            let diagnostic_id = self.next_diagnostic_id()?;
            let pipeline = IngestPipeline::new(AllowPolicy, self.provider_classifier.as_ref());
            let outcome =
                pipeline.process(raw_event, matched, event_id, label_event_id, diagnostic_id);

            if let Some(trace_id) = matched_trace_id {
                self.mark_semantic_projection_dirty(trace_id);
                if outcome
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.kind == DiagnosticKind::RuntimeFatal)
                {
                    trace_runtime
                        .fail_trace(trace_id, observed_at)
                        .map_err(|error| ControlError::new("fail_trace", format!("{error:?}")))?;
                }
                batch.trace_ids.insert(trace_id);
                for event in outcome.events {
                    let output = self.semantic_actions.observe_event(&event);
                    let retain_event = output.retain_event;
                    semantic_action_count =
                        semantic_action_count.saturating_add(output.actions.len());
                    semantic_link_count = semantic_link_count.saturating_add(output.links.len());
                    batch
                        .semantic_actions
                        .extend(SemanticActionBatch::from_action_output(
                            output.actions,
                            output.links,
                            output.file_observation_paths,
                            output.file_path_sets,
                            output.llm_request_contents,
                        ));
                    retained_events = retained_events.saturating_add(output.deferred_events.len());
                    for deferred_event in output.deferred_events {
                        batch
                            .events
                            .push(self.prepare_event_for_storage(deferred_event));
                    }
                    if retain_event {
                        retained_events = retained_events.saturating_add(1);
                        batch.events.push(self.prepare_event_for_storage(event));
                    }
                }
            }
            batch.diagnostics.extend(outcome.diagnostics);
        }

        self.workload_diagnostics.record_event_projection(
            input_events,
            retained_events,
            semantic_action_count,
            semantic_link_count,
        );
        self.persist_live_event_batch(trace_runtime, batch)
    }

    pub(super) fn persist_observed_event_batch(
        &mut self,
        trace_runtime: &TraceRuntime,
        events: Vec<DomainEvent>,
    ) -> Result<(), ControlError> {
        self.persist_observed_event_batch_with_process_records(trace_runtime, events, Vec::new())
    }

    pub(super) fn persist_observed_event_batch_with_process_records(
        &mut self,
        trace_runtime: &TraceRuntime,
        events: Vec<DomainEvent>,
        process_records: Vec<ProcessRecord>,
    ) -> Result<(), ControlError> {
        if events.is_empty() && process_records.is_empty() {
            return Ok(());
        }
        let input_events = events.len();
        let mut retained_events = 0usize;
        let mut semantic_action_count = 0usize;
        let mut semantic_link_count = 0usize;
        let mut batch = LiveEventBatch {
            process_records: process_records
                .into_iter()
                .map(|record| (record.identity, record))
                .collect(),
            ..LiveEventBatch::default()
        };
        for event in events {
            self.mark_semantic_projection_dirty(event.envelope.trace_id);
            let output = self.semantic_actions.observe_event(&event);
            let retain_event = output.retain_event;
            semantic_action_count = semantic_action_count.saturating_add(output.actions.len());
            semantic_link_count = semantic_link_count.saturating_add(output.links.len());
            batch
                .semantic_actions
                .extend(SemanticActionBatch::from_action_output(
                    output.actions,
                    output.links,
                    output.file_observation_paths,
                    output.file_path_sets,
                    output.llm_request_contents,
                ));
            retained_events = retained_events.saturating_add(output.deferred_events.len());
            for deferred_event in output.deferred_events {
                batch
                    .events
                    .push(self.prepare_event_for_storage(deferred_event));
            }
            if retain_event {
                retained_events = retained_events.saturating_add(1);
                batch.events.push(self.prepare_event_for_storage(event));
            }
        }
        self.workload_diagnostics.record_event_projection(
            input_events,
            retained_events,
            semantic_action_count,
            semantic_link_count,
        );
        self.persist_live_event_batch(trace_runtime, batch)
    }

    fn persist_live_event_batch(
        &mut self,
        trace_runtime: &TraceRuntime,
        batch: LiveEventBatch,
    ) -> Result<(), ControlError> {
        let trace_states = self.trace_states_for_persistence(trace_runtime, batch.trace_ids)?;
        self.persist_observed_batch_then_publish(
            trace_runtime,
            batch.events,
            batch.diagnostics,
            batch.semantic_actions,
            trace_states,
            batch.process_records.into_values().collect(),
        )
    }

    fn trace_states_for_persistence(
        &self,
        trace_runtime: &TraceRuntime,
        trace_ids: BTreeSet<TraceId>,
    ) -> Result<Vec<TraceStateRecord>, ControlError> {
        trace_ids
            .into_iter()
            .map(|trace_id| self.trace_state_record_for_persistence(trace_runtime, trace_id))
            .collect()
    }

    fn prepare_event_for_storage(&self, mut event: DomainEvent) -> DomainEvent {
        if !self.file_observation.enabled
            || self.file_observation.metadata_retention == FileMetadataRetention::Full
        {
            return event;
        }
        let EventPayload::File(payload) = &mut event.payload else {
            return event;
        };
        payload.metadata.remove("operation");
        payload.metadata.remove("result");
        remove_if_matches_path(&mut payload.metadata, "raw_path", payload.path.as_deref());
        remove_if_matches_path(&mut payload.metadata, "fd_target", payload.path.as_deref());
        let target_path = payload.metadata.get("target_path").cloned();
        remove_if_matches_path(
            &mut payload.metadata,
            "raw_target_path",
            target_path.as_deref(),
        );
        event
    }
}

fn remove_if_matches_path(
    metadata: &mut std::collections::BTreeMap<String, String>,
    key: &str,
    canonical: Option<&str>,
) {
    if canonical.is_some_and(|canonical| {
        metadata
            .get(key)
            .is_some_and(|candidate| candidate == canonical)
    }) {
        metadata.remove(key);
    }
}

#[derive(Default)]
struct LiveEventBatch {
    events: Vec<DomainEvent>,
    diagnostics: Vec<DiagnosticRecord>,
    semantic_actions: SemanticActionBatch,
    trace_ids: BTreeSet<TraceId>,
    process_records: BTreeMap<ProcessIdentity, ProcessRecord>,
}
