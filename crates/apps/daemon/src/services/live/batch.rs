//! Batched live-event persistence for deferred seccomp observations.

use std::collections::BTreeSet;

use collector_event::{RawCollectorEvent, RawObservationPayload};
use collector_instance::CollectorInstance;
use control_contract::reply::ControlError;
use ingest_runtime::IngestPipeline;
use model_core::diagnostics::{DiagnosticKind, DiagnosticRecord};
use model_core::event::DomainEvent;
use model_core::ids::TraceId;
use model_core::process::ProcessMembership;
use model_core::trace::{TraceLifecycleState, TraceRecord};
use plugin_policy_host::engine::PluginPolicyEngine;
use plugin_policy_host::registry::PluginRegistry;
use semantic_action::{SemanticAction, SemanticActionLink, SemanticActionWriteStore};
use store_tx_contract::boundary::TransactionBoundary;
use store_write_contract::diagnostics::DiagnosticWriteStore;
use store_write_contract::events::EventWriteStore;
use store_write_contract::memberships::MembershipWriteStore;
use store_write_contract::traces::TraceWriteStore;
use trace_runtime::registry::TraceRuntime;

use crate::services::attach::SqliteAttachService;

impl SqliteAttachService {
    pub(super) fn process_live_event_batch(
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
            let matched = self.apply_runtime_effects(trace_runtime, &raw_event)?;
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
                    let semantic_output = self.semantic_actions.observe_event(&event);
                    batch.semantic_actions.extend(semantic_output.actions);
                    batch.semantic_action_links.extend(semantic_output.links);
                    batch.events.push(event);
                }
            }
            batch.diagnostics.extend(outcome.diagnostics);
        }

        self.persist_live_event_batch(trace_runtime, batch)
    }

    fn persist_live_event_batch(
        &mut self,
        trace_runtime: &TraceRuntime,
        batch: LiveEventBatch,
    ) -> Result<(), ControlError> {
        let transaction = self
            .storage
            .begin()
            .map_err(|error| ControlError::new(error.stage, error.message))?;
        let write_result = self.write_live_event_batch(trace_runtime, batch);
        match write_result {
            Ok(commit_result) => {
                transaction
                    .commit()
                    .map_err(|error| ControlError::new(error.stage, error.message))?;
                self.publish_live_otel_actions(
                    trace_runtime,
                    &commit_result.semantic_actions,
                    &commit_result.semantic_action_links,
                )?;
                for trace_id in commit_result.terminal_trace_ids {
                    self.collector
                        .unbind_trace(trace_id)
                        .map_err(|error| ControlError::new(error.stage, error.message))?;
                }
                Ok(())
            }
            Err(error) => {
                let _ = transaction.rollback();
                Err(error)
            }
        }
    }

    fn write_live_event_batch(
        &mut self,
        trace_runtime: &TraceRuntime,
        batch: LiveEventBatch,
    ) -> Result<LiveEventCommitResult, ControlError> {
        for event in batch.events {
            self.storage
                .append_event(event)
                .map_err(|error| ControlError::new(error.stage, error.message))?;
        }
        for action in batch.semantic_actions.iter().cloned() {
            self.storage
                .upsert_semantic_action(action)
                .map_err(|error| ControlError::new(error.stage, error.message))?;
        }
        for link in batch.semantic_action_links.iter().cloned() {
            self.storage
                .upsert_semantic_action_link(link)
                .map_err(|error| ControlError::new(error.stage, error.message))?;
        }
        for diagnostic in batch.diagnostics {
            self.storage
                .append_diagnostic(diagnostic)
                .map_err(|error| ControlError::new(error.stage, error.message))?;
        }
        let terminal_trace_ids = self.write_trace_states(trace_runtime, batch.trace_ids)?;
        Ok(LiveEventCommitResult {
            terminal_trace_ids,
            semantic_actions: batch.semantic_actions,
            semantic_action_links: batch.semantic_action_links,
        })
    }

    fn write_trace_states(
        &mut self,
        trace_runtime: &TraceRuntime,
        trace_ids: BTreeSet<TraceId>,
    ) -> Result<Vec<TraceId>, ControlError> {
        let mut terminal_trace_ids = Vec::new();
        for trace_id in trace_ids {
            let state = trace_state_for_persistence(trace_runtime, trace_id)?;
            self.storage
                .create_trace(state.trace)
                .map_err(|error| ControlError::new(error.stage, error.message))?;
            for membership in state.memberships {
                self.storage
                    .upsert_membership(membership)
                    .map_err(|error| ControlError::new(error.stage, error.message))?;
            }
            if state.terminal {
                terminal_trace_ids.push(trace_id);
            }
        }
        Ok(terminal_trace_ids)
    }
}

#[derive(Default)]
struct LiveEventBatch {
    events: Vec<DomainEvent>,
    diagnostics: Vec<DiagnosticRecord>,
    semantic_actions: Vec<SemanticAction>,
    semantic_action_links: Vec<SemanticActionLink>,
    trace_ids: BTreeSet<TraceId>,
}

struct LiveEventCommitResult {
    terminal_trace_ids: Vec<TraceId>,
    semantic_actions: Vec<SemanticAction>,
    semantic_action_links: Vec<SemanticActionLink>,
}

struct TraceStatePersistence {
    trace: TraceRecord,
    memberships: Vec<ProcessMembership>,
    terminal: bool,
}

fn trace_state_for_persistence(
    trace_runtime: &TraceRuntime,
    trace_id: TraceId,
) -> Result<TraceStatePersistence, ControlError> {
    trace_runtime
        .get_trace(trace_id)
        .map(|entry| TraceStatePersistence {
            trace: entry.trace.clone(),
            memberships: entry.memberships.memberships().cloned().collect(),
            terminal: matches!(
                entry.trace.lifecycle_state,
                TraceLifecycleState::Completed | TraceLifecycleState::Failed
            ),
        })
        .ok_or_else(|| ControlError::new("persist_trace_state", "trace not found"))
}
