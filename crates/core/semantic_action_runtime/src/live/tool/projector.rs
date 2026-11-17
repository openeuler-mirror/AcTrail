//! LLM tool-call, result, and agent-invocation projection façade.

use std::collections::BTreeMap;
use std::time::SystemTime;

use config_core::daemon::AgentInvocationConfig;
use model_core::diagnostics::LlmPipelineDiagnosticCode;
use model_core::ids::TraceId;
use semantic_action::{SemanticAction, SemanticActionKind, attr_keys as attrs};

use super::internal::correlation::AgentInvocationCorrelator;
use super::internal::declaration::{DeclaredLlmToolCall, DeclaredLlmToolCalls};
use super::internal::emission::ToolSemanticEmitter;
use super::internal::state::{StateMutation, ToolInteractionState, ToolResultOutcome};
use super::{ToolProjectionBatch, ToolProjectionOutput};
use crate::llm_pipeline::{ProjectedLlmToolCall, ProjectedLlmToolResult};

pub(in crate::live) struct ToolInteractionProjector {
    correlator: AgentInvocationCorrelator,
    state: ToolInteractionState,
}

impl ToolInteractionProjector {
    pub(in crate::live) fn new(config: AgentInvocationConfig, max_entries_per_trace: u32) -> Self {
        Self {
            correlator: AgentInvocationCorrelator::new(config),
            state: ToolInteractionState::new(max_entries_per_trace),
        }
    }

    pub(in crate::live) fn project(
        &mut self,
        batch: ToolProjectionBatch<'_>,
    ) -> ToolProjectionOutput {
        let mut emitter = ToolSemanticEmitter::new(batch.request_lineages);
        for response in batch
            .actions
            .iter()
            .chain(batch.updated_actions.iter())
            .filter(|action| action.kind == SemanticActionKind::LlmResponse)
        {
            self.project_response(response, &mut emitter);
        }

        let requests = batch
            .actions
            .iter()
            .chain(batch.updated_actions.iter())
            .filter(|action| action.kind == SemanticActionKind::LlmRequest)
            .map(|action| (action.action_id.as_str(), action))
            .collect::<BTreeMap<_, _>>();
        for result in batch.tool_results {
            let Some(request) = requests.get(result.request_action_id.as_str()) else {
                emitter.diagnose_data_discarded(
                    result.trace_id,
                    &result.process,
                    result.observed_at,
                );
                continue;
            };
            self.project_result(request, result, &mut emitter);
        }
        emitter.finish()
    }

    pub(in crate::live) fn finish_trace(
        &mut self,
        trace_id: TraceId,
        finished_at: SystemTime,
    ) -> ToolProjectionOutput {
        let mut emitter = ToolSemanticEmitter::empty();
        for finalized in self.state.finish_trace(trace_id, finished_at) {
            emitter.emit_finalized_invocation(finalized);
        }
        emitter.finish()
    }

    pub(in crate::live) fn forget_trace(&mut self, trace_id: TraceId) {
        self.state.forget_trace(trace_id);
    }

    fn project_response(&mut self, response: &SemanticAction, emitter: &mut ToolSemanticEmitter) {
        let declared_calls = DeclaredLlmToolCalls::from_response(response);
        if declared_calls.malformed {
            emitter.diagnose_decode_failed(
                response,
                declared_calls.discarded_entries,
                LlmPipelineDiagnosticCode::ToolCallsJsonInvalid,
            );
        } else if declared_calls.discarded_entries > 0 {
            emitter.diagnose_decode_failed(
                response,
                declared_calls.discarded_entries,
                LlmPipelineDiagnosticCode::ToolCallNameMissing,
            );
        }
        for declared in declared_calls.calls {
            let action = emitter.tool_call_action(response, &declared);
            let StateMutation {
                changed,
                existing,
                eviction,
            } = self
                .state
                .record_tool_call(action.clone(), declared.tool_call_id.clone());
            emitter.emit_capacity_eviction(
                action.trace_id,
                &action.process,
                action.start_time,
                eviction,
            );
            emitter.emit_tool_call(response, &action, changed, existing);
            if self.correlator.projects_invocation(&declared.name) {
                self.project_agent_invocation(&action, &declared, emitter);
            }
        }
    }

    fn project_agent_invocation(
        &mut self,
        tool_call: &SemanticAction,
        declared: &DeclaredLlmToolCall,
        emitter: &mut ToolSemanticEmitter,
    ) {
        if self
            .state
            .has_agent_invocation(tool_call.trace_id, &tool_call.action_id)
        {
            return;
        }
        let invocation = emitter.agent_invocation_action(tool_call, declared);
        let StateMutation {
            changed, eviction, ..
        } = self
            .state
            .record_agent_invocation(&tool_call.action_id, &invocation);
        emitter.emit_capacity_eviction(
            invocation.trace_id,
            &invocation.process,
            invocation.start_time,
            eviction,
        );
        if !changed {
            return;
        }
        emitter.emit_agent_invocation(tool_call, invocation);
        // The result may already have been processed; the invocation is then complete.
        let early_result = declared.tool_call_id.as_deref().and_then(|id| {
            self.state
                .take_early_result(tool_call.trace_id, id, &tool_call.process)
        });
        if let Some(outcome) = early_result
            && let Some(completed) = self.state.complete_agent_invocation(
                tool_call.trace_id,
                &tool_call.action_id,
                outcome,
            )
        {
            emitter.emit_agent_invocation(tool_call, completed);
        }
    }

    fn project_result(
        &mut self,
        request: &SemanticAction,
        result: &ProjectedLlmToolResult,
        emitter: &mut ToolSemanticEmitter,
    ) {
        let observed_calls = result.tool_call_id.as_deref().map_or_else(Vec::new, |id| {
            self.state
                .observed_tool_calls(request.trace_id, id, &request.process)
        });
        let tool_call = Self::tool_call_of_result(result, &observed_calls);
        let action = emitter.tool_result_action(request, result, tool_call.as_ref());
        let StateMutation {
            changed,
            existing,
            eviction,
        } = self.state.record_tool_result(action.clone());
        emitter.emit_capacity_eviction(
            action.trace_id,
            &action.process,
            action.start_time,
            eviction,
        );
        emitter.emit_tool_result(request, &action, changed, existing);
        self.complete_invocation(
            request,
            result,
            tool_call.as_ref(),
            &observed_calls,
            emitter,
        );
    }

    /// The call a result answers comes from the request that carries the
    /// result, which repeats the model's latest output. Requests that omit the
    /// history fall back to the latest call observed in a response. A result
    /// without an id, or whose call is in neither place, carries no call.
    fn tool_call_of_result(
        result: &ProjectedLlmToolResult,
        observed_calls: &[SemanticAction],
    ) -> Option<ProjectedLlmToolCall> {
        result.tool_call_id.as_ref()?;
        if let Some(tool_call) = &result.tool_call {
            return Some(tool_call.clone());
        }
        let observed = observed_calls.last()?;
        Some(ProjectedLlmToolCall {
            name: observed.attributes.get(attrs::llm_tool_call::NAME)?.clone(),
        })
    }

    /// Completes the agent invocation the answered call started. A result can
    /// be processed before its call is declared; its outcome is then kept until
    /// the call arrives.
    fn complete_invocation(
        &mut self,
        request: &SemanticAction,
        result: &ProjectedLlmToolResult,
        tool_call: Option<&ProjectedLlmToolCall>,
        observed_calls: &[SemanticAction],
        emitter: &mut ToolSemanticEmitter,
    ) {
        let Some(tool_call_id) = result.tool_call_id.as_deref() else {
            return;
        };
        let outcome = ToolResultOutcome::new(request, result.is_error);
        match observed_calls {
            [observed] => {
                if let Some(invocation) = self.state.complete_agent_invocation(
                    request.trace_id,
                    &observed.action_id,
                    outcome,
                ) {
                    emitter.emit_agent_invocation(observed, invocation);
                }
            }
            [] if tool_call.is_none_or(|call| self.correlator.projects_invocation(&call.name)) => {
                self.state.remember_early_result(
                    request.trace_id,
                    tool_call_id,
                    request.process,
                    outcome,
                );
            }
            _ => {}
        }
    }
}
