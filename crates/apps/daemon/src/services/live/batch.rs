//! Batched live-event persistence for deferred seccomp observations.

use std::collections::BTreeSet;

use collector_event::{RawCollectorEvent, RawObservationPayload};
use control_contract::reply::ControlError;
use ingest_runtime::IngestPipeline;
use model_core::diagnostics::{DiagnosticKind, DiagnosticRecord};
use model_core::event::DomainEvent;
use model_core::ids::TraceId;
use plugin_policy_host::engine::PluginPolicyEngine;
use plugin_policy_host::registry::PluginRegistry;
use recording_runtime::{SemanticActionBatch, TraceStateRecord};
use trace_runtime::registry::TraceRuntime;

use crate::services::attach::StorageAttachService;
use crate::services::identity::TraceIdentityResolver;

impl StorageAttachService {
    pub(in crate::services) fn process_live_event_batch(
        &mut self,
        trace_runtime: &mut TraceRuntime,
        raw_events: Vec<RawCollectorEvent>,
    ) -> Result<(), ControlError> {
        if raw_events.is_empty() {
            return Ok(());
        }
        let mut batch = LiveEventBatch::default();
        for raw_event in raw_events {
            let observed_at = raw_event.envelope.observed_at;
            let matched = TraceIdentityResolver::apply_runtime_effects(trace_runtime, &raw_event)?;
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
            let pipeline = IngestPipeline::new(
                PluginPolicyEngine::new(PluginRegistry::new()),
                self.provider_classifier.as_ref(),
            );
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
                    batch
                        .semantic_actions
                        .extend(self.observe_semantic_actions_for_event(&event));
                    batch.events.push(event);
                }
            }
            batch.diagnostics.extend(outcome.diagnostics);
        }

        self.persist_live_event_batch(trace_runtime, batch)
    }

    pub(super) fn persist_observed_event_batch(
        &mut self,
        trace_runtime: &TraceRuntime,
        events: Vec<DomainEvent>,
    ) -> Result<(), ControlError> {
        if events.is_empty() {
            return Ok(());
        }
        let mut batch = LiveEventBatch::default();
        for event in events {
            self.mark_semantic_projection_dirty(event.envelope.trace_id);
            batch
                .semantic_actions
                .extend(self.observe_semantic_actions_for_event(&event));
            batch.events.push(event);
        }
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
}

#[derive(Default)]
struct LiveEventBatch {
    events: Vec<DomainEvent>,
    diagnostics: Vec<DiagnosticRecord>,
    semantic_actions: SemanticActionBatch,
    trace_ids: BTreeSet<TraceId>,
}
