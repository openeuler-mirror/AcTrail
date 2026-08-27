//! Per-trace admission and fail-local eviction for correlation streams.

use std::time::SystemTime;

use model_core::diagnostics::{
    LlmPipelineDiagnostic, LlmPipelineDiagnosticCode, LlmPipelineDiagnosticSeverity,
    LlmPipelineDiagnosticStage,
};
use semantic_action::{SemanticActionCompleteness, SemanticActionStatus};

use crate::llm_pipeline::projection::correlation::{self as call, LlmStreamKey, StreamAdmission};

use super::super::ProjectionBatch;
use super::ProjectionCoordinator;

impl ProjectionCoordinator {
    pub(super) fn admit_correlation_stream(
        &mut self,
        stream_key: &LlmStreamKey,
        observed_at: SystemTime,
        output: &mut ProjectionBatch,
    ) -> bool {
        let evicted = match self.correlation.stream_owners.admit(stream_key) {
            StreamAdmission::Inserted | StreamAdmission::Existing => return true,
            StreamAdmission::Evicted(evicted) => evicted,
            StreamAdmission::SequenceExhausted => {
                output.diagnostics.push(correlation_stream_diagnostic(
                    stream_key,
                    observed_at,
                    1,
                    LlmPipelineDiagnosticCode::CorrelationSequenceExhausted,
                ));
                return false;
            }
        };
        let mut discarded_entries = 0_u64;
        if let Some(requests) = self.correlation.open_requests.remove(&evicted) {
            discarded_entries =
                discarded_entries.saturating_add(u64::try_from(requests.len()).unwrap_or(u64::MAX));
            for request in requests {
                let mut partial_call = call::llm_call_from_request_response(&request.action, None);
                partial_call.status = SemanticActionStatus::Error;
                partial_call.completeness = SemanticActionCompleteness::Partial;
                partial_call.end_time = Some(observed_at);
                self.push_recorded_action(partial_call, output);
            }
        }
        discarded_entries = discarded_entries.saturating_add(
            self.correlation
                .pending_responses
                .remove(&evicted)
                .map_or(0, |responses| {
                    u64::try_from(responses.len()).unwrap_or(u64::MAX)
                }),
        );
        discarded_entries = discarded_entries.saturating_add(
            self.correlation
                .confirmed_http_exchanges
                .remove(&evicted)
                .map_or(0, |exchanges| {
                    u64::try_from(exchanges.len()).unwrap_or(u64::MAX)
                }),
        );
        discarded_entries = discarded_entries.saturating_add(u64::from(
            self.correlation.http_exchange_streams.remove(&evicted),
        ));
        discarded_entries = discarded_entries.saturating_add(u64::from(
            self.correlation
                .incomplete_http1_responses
                .remove(&evicted)
                .is_some(),
        ));
        discarded_entries = discarded_entries.saturating_add(
            self.correlation
                .damaged_http_responses
                .remove(&evicted)
                .map_or(0, |responses| {
                    u64::try_from(responses.len()).unwrap_or(u64::MAX)
                }),
        );
        if let Some(localized) = self
            .correlation
            .localized_http1_request_outputs
            .remove(&evicted)
        {
            output.extend(localized);
        }
        let damaged_bindings = self.correlation.damaged_binding_owners.take_owner(
            evicted.trace_id,
            evicted.process,
            &evicted.stream_key,
        );
        discarded_entries = discarded_entries
            .saturating_add(u64::try_from(damaged_bindings.len()).unwrap_or(u64::MAX));
        for key in damaged_bindings {
            self.correlation.damaged_response_bindings.remove(&key);
        }
        let late_failure_bindings = self.correlation.late_failure_binding_owners.take_owner(
            evicted.trace_id,
            evicted.process,
            &evicted.stream_key,
        );
        discarded_entries = discarded_entries
            .saturating_add(u64::try_from(late_failure_bindings.len()).unwrap_or(u64::MAX));
        for key in late_failure_bindings {
            self.correlation.late_http_failure_bindings.remove(&key);
        }
        let active_bindings = self.correlation.active_binding_owners.take_owner(
            evicted.trace_id,
            evicted.process,
            &evicted.stream_key,
        );
        discarded_entries = discarded_entries
            .saturating_add(u64::try_from(active_bindings.len()).unwrap_or(u64::MAX));
        for key in active_bindings {
            if let Some(binding) = self.correlation.active_response_requests.remove(&key) {
                let mut partial_call = call::llm_call_from_request_response(&binding.request, None);
                partial_call.status = SemanticActionStatus::Error;
                partial_call.completeness = SemanticActionCompleteness::Partial;
                partial_call.end_time = Some(observed_at);
                self.push_recorded_action(partial_call, output);
            }
        }
        output.diagnostics.push(correlation_stream_diagnostic(
            &evicted,
            observed_at,
            discarded_entries.max(1),
            LlmPipelineDiagnosticCode::CorrelationStreamCapacityEvicted,
        ));
        true
    }
}

fn correlation_stream_diagnostic(
    stream_key: &LlmStreamKey,
    observed_at: SystemTime,
    discarded_entries: u64,
    code: LlmPipelineDiagnosticCode,
) -> LlmPipelineDiagnostic {
    LlmPipelineDiagnostic::new(
        stream_key.trace_id,
        &stream_key.process,
        observed_at,
        code,
        LlmPipelineDiagnosticSeverity::Warning,
        LlmPipelineDiagnosticStage::Correlation,
    )
    .with_stream_key(&stream_key.stream_key)
    .with_discarded_entries(discarded_entries)
}
