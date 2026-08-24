//! Boundary types exchanged with the live semantic action runtime.

use model_core::diagnostics::LlmPipelineDiagnostic;
use semantic_action::{LlmRequestLineageWrite, SemanticAction, SemanticActionLink};

use crate::llm_pipeline::ProjectedLlmToolResult;

pub(in crate::live) struct ToolProjectionBatch<'a> {
    pub(in crate::live) actions: &'a [SemanticAction],
    pub(in crate::live) tool_results: &'a [ProjectedLlmToolResult],
    pub(in crate::live) request_lineages: &'a [LlmRequestLineageWrite],
}

#[derive(Default)]
pub(in crate::live) struct ToolProjectionOutput {
    pub(in crate::live) actions: Vec<SemanticAction>,
    pub(in crate::live) links: Vec<SemanticActionLink>,
    pub(in crate::live) diagnostics: Vec<LlmPipelineDiagnostic>,
}
