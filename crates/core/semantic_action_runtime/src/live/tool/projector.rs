//! LLM tool-call, result, and agent-invocation projection façade.

use std::collections::BTreeMap;
use std::time::SystemTime;

use config_core::daemon::AgentInvocationConfig;
use model_core::diagnostics::LlmPipelineDiagnosticCode;
use model_core::ids::TraceId;
use semantic_action::{SemanticAction, SemanticActionKind};

use super::internal::correlation::{
    AgentInvocationCorrelator, ToolResultBinding, ToolResultBindingState,
};
use super::internal::declaration::{DeclaredLlmToolCall, DeclaredLlmToolCalls};
use super::internal::emission::ToolSemanticEmitter;
use super::internal::state::{StateMutation, ToolInteractionState};
use super::{ToolProjectionBatch, ToolProjectionOutput};

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
            .filter(|action| action.kind == SemanticActionKind::LlmResponse)
        {
            self.project_response(response, &mut emitter);
        }

        let requests = batch
            .actions
            .iter()
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
        for request in requests.values() {
            let continuing_trajectory = batch.request_lineages.iter().any(|lineage| {
                lineage.action_id == request.action_id && lineage.parent_action_id.is_some()
            });
            if let Some(invocation) =
                self.correlator
                    .link_agent_child(&mut self.state, request, continuing_trajectory)
            {
                emitter.emit_agent_child(&invocation, request);
            }
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
            let StateMutation { changed, eviction } = self
                .state
                .record_tool_call(action.clone(), declared.tool_call_id.clone());
            emitter.emit_capacity_eviction(
                action.trace_id,
                &action.process,
                action.start_time,
                eviction,
            );
            emitter.emit_tool_call(response, &action, changed);
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
        let (prompt_message_hashes, prompt_preview) =
            self.correlator.invocation_prompt_key(declared);
        let StateMutation { changed, eviction } = self.state.record_agent_invocation(
            &tool_call.action_id,
            &invocation,
            prompt_message_hashes,
            prompt_preview,
        );
        emitter.emit_capacity_eviction(
            invocation.trace_id,
            &invocation.process,
            invocation.start_time,
            eviction,
        );
        if changed {
            emitter.emit_agent_invocation(tool_call, invocation);
        }
    }

    fn project_result(
        &mut self,
        request: &SemanticAction,
        result: &crate::llm_pipeline::ProjectedLlmToolResult,
        emitter: &mut ToolSemanticEmitter,
    ) {
        let binding =
            self.correlator
                .bind_tool_result(&self.state, request, result.tool_call_id.as_deref());
        let binding_diagnostic = match binding.state {
            ToolResultBindingState::MissingId => {
                Some(LlmPipelineDiagnosticCode::ToolResultCallIdMissing)
            }
            ToolResultBindingState::Unmatched => {
                Some(LlmPipelineDiagnosticCode::ToolResultCallUnmatched)
            }
            ToolResultBindingState::Ambiguous => {
                Some(LlmPipelineDiagnosticCode::ToolResultCallAmbiguous)
            }
            ToolResultBindingState::Bound => None,
        };
        if let Some(code) = binding_diagnostic {
            emitter.diagnose_lifecycle_gap(request, code);
        }
        let action = emitter.tool_result_action(request, result, binding.state.as_str());
        let StateMutation { changed, eviction } = self.state.record_tool_result(action.clone());
        emitter.emit_capacity_eviction(
            action.trace_id,
            &action.process,
            action.start_time,
            eviction,
        );
        self.emit_bound_result(request, result.is_error, action, changed, binding, emitter);
    }

    fn emit_bound_result(
        &mut self,
        request: &SemanticAction,
        is_error: bool,
        result_action: SemanticAction,
        changed: bool,
        binding: ToolResultBinding,
        emitter: &mut ToolSemanticEmitter,
    ) {
        let tool_call = binding
            .tool_call_action_id
            .as_deref()
            .and_then(|action_id| self.state.tool_call_action(request.trace_id, action_id));
        emitter.emit_tool_result(tool_call.as_ref(), &result_action, changed);
        let Some(tool_call_action_id) = binding.tool_call_action_id else {
            return;
        };
        let Some(tool_call) = tool_call else {
            return;
        };
        if let Some(invocation) = self.state.complete_agent_invocation(
            request.trace_id,
            &tool_call_action_id,
            is_error,
            request,
        ) {
            emitter.emit_agent_invocation(&tool_call, invocation);
        }
    }
}
