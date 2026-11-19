//! Stateful accumulation of one tool projection output batch.

use std::time::SystemTime;

use model_core::diagnostics::{LlmPipelineDiagnosticCode, LlmPipelineDiagnosticSeverity};
use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;
use semantic_action::{
    LlmRequestLineageWrite, SemanticAction, SemanticActionLinkOrigin, SemanticActionLinkRole,
};

use crate::live::tool::contract::ToolProjectionOutput;
use crate::live::tool::internal::declaration::DeclaredLlmToolCall;
use crate::live::tool::internal::state::{FinalizedInvocation, StateEviction};
use crate::llm_pipeline::{ProjectedLlmToolCall, ProjectedLlmToolResult};

use super::actions::{
    agent_invocation_action, tool_call_action, tool_diagnostic, tool_result_action,
};
use super::links::{action_link, lineage_links};

pub(in crate::live::tool) struct ToolSemanticEmitter {
    output: ToolProjectionOutput,
}

impl ToolSemanticEmitter {
    pub(in crate::live::tool) fn new(lineages: &[LlmRequestLineageWrite]) -> Self {
        let mut output = ToolProjectionOutput::default();
        output.links.extend(lineage_links(lineages));
        Self { output }
    }

    pub(in crate::live::tool) fn empty() -> Self {
        Self {
            output: ToolProjectionOutput::default(),
        }
    }

    pub(in crate::live::tool) fn tool_call_action(
        &self,
        response: &SemanticAction,
        declared: &DeclaredLlmToolCall,
    ) -> SemanticAction {
        tool_call_action(response, declared)
    }

    pub(in crate::live::tool) fn tool_result_action(
        &self,
        request: &SemanticAction,
        result: &ProjectedLlmToolResult,
        tool_call: Option<&ProjectedLlmToolCall>,
    ) -> SemanticAction {
        tool_result_action(request, result, tool_call)
    }

    pub(in crate::live::tool) fn agent_invocation_action(
        &self,
        tool_call: &SemanticAction,
        declared: &DeclaredLlmToolCall,
    ) -> SemanticAction {
        agent_invocation_action(tool_call, declared)
    }

    pub(in crate::live::tool) fn emit_tool_call(
        &mut self,
        response: &SemanticAction,
        action: &SemanticAction,
        changed: bool,
        existing: bool,
    ) {
        if !changed {
            return;
        }
        if existing {
            self.output
                .updates
                .push(crate::live::ActionUpdateFactory::lifecycle(action, None));
            self.output.updated_actions.push(action.clone());
        } else {
            self.output.actions.push(action.clone());
        }
        self.output.links.push(action_link(
            response,
            action,
            SemanticActionLinkRole::LlmResponseToolCall,
            SemanticActionLinkOrigin::Observed,
        ));
    }

    pub(in crate::live::tool) fn emit_tool_result(
        &mut self,
        request: &SemanticAction,
        action: &SemanticAction,
        changed: bool,
        existing: bool,
    ) {
        if !changed {
            return;
        }
        if existing {
            self.output
                .updates
                .push(crate::live::ActionUpdateFactory::lifecycle(action, None));
            self.output.updated_actions.push(action.clone());
        } else {
            self.output.actions.push(action.clone());
        }
        self.output.links.push(action_link(
            request,
            action,
            SemanticActionLinkRole::LlmRequestToolResult,
            SemanticActionLinkOrigin::Observed,
        ));
    }

    pub(in crate::live::tool) fn emit_agent_invocation(
        &mut self,
        tool_call: &SemanticAction,
        invocation: SemanticAction,
    ) {
        self.output.links.push(action_link(
            tool_call,
            &invocation,
            SemanticActionLinkRole::LlmToolCallAgentInvocation,
            SemanticActionLinkOrigin::Observed,
        ));
        if invocation.status == semantic_action::SemanticActionStatus::InProgress {
            self.output.actions.push(invocation);
        } else {
            self.output
                .updates
                .push(crate::live::ActionUpdateFactory::lifecycle(
                    &invocation,
                    None,
                ));
            self.output.updated_actions.push(invocation);
        }
    }

    pub(in crate::live::tool) fn emit_finalized_invocation(
        &mut self,
        finalized: FinalizedInvocation,
    ) {
        self.output
            .updates
            .push(crate::live::ActionUpdateFactory::lifecycle(
                &finalized.action,
                Some(semantic_action::SemanticActionFinalizationReason::TraceClosed),
            ));
        self.output.updated_actions.push(finalized.action.clone());
        if let Some(tool_call) = finalized.tool_call {
            self.output.links.push(action_link(
                &tool_call,
                &finalized.action,
                SemanticActionLinkRole::LlmToolCallAgentInvocation,
                SemanticActionLinkOrigin::Observed,
            ));
        }
    }

    pub(in crate::live::tool) fn diagnose_decode_failed(
        &mut self,
        action: &SemanticAction,
        discarded_entries: u64,
        code: LlmPipelineDiagnosticCode,
    ) {
        self.output.diagnostics.push(
            tool_diagnostic(
                action.trace_id,
                &action.process,
                action.start_time,
                code,
                LlmPipelineDiagnosticSeverity::Error,
            )
            .with_discarded_entries(discarded_entries),
        );
    }

    pub(in crate::live::tool) fn diagnose_data_discarded(
        &mut self,
        trace_id: TraceId,
        process: &ProcessIdentity,
        observed_at: SystemTime,
    ) {
        self.output.diagnostics.push(
            tool_diagnostic(
                trace_id,
                process,
                observed_at,
                LlmPipelineDiagnosticCode::ToolResultRequestMissing,
                LlmPipelineDiagnosticSeverity::Error,
            )
            .with_discarded_entries(1),
        );
    }

    pub(in crate::live::tool) fn emit_capacity_eviction(
        &mut self,
        trace_id: TraceId,
        process: &ProcessIdentity,
        observed_at: SystemTime,
        eviction: Option<StateEviction>,
    ) {
        let Some(eviction) = eviction else {
            return;
        };
        self.output.diagnostics.push(
            tool_diagnostic(
                trace_id,
                process,
                observed_at,
                LlmPipelineDiagnosticCode::ToolStateCapacityEvicted,
                LlmPipelineDiagnosticSeverity::Warning,
            )
            .with_discarded_entries(1),
        );
        if let Some(invocation) = eviction.materialized_invocation {
            self.output
                .updates
                .push(crate::live::ActionUpdateFactory::lifecycle(
                    &invocation,
                    Some(semantic_action::SemanticActionFinalizationReason::CapacityEvicted),
                ));
            self.output.updated_actions.push(invocation);
        }
    }

    pub(in crate::live::tool) fn finish(self) -> ToolProjectionOutput {
        self.output
    }
}
