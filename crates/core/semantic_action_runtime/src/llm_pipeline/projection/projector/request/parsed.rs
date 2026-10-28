use config_core::daemon::SemanticRetentionConfig;
use semantic_action::{
    SemanticAction, SemanticActionCompleteness, SemanticActionKind, SemanticActionStatus,
    attr_keys as attrs,
};
use serde_json::Value;

use super::super::parsed::ParsedMessageContext;
use super::{
    LlmRequestBody, LlmRequestFacts, LlmRequestObservation, LlmRequestParserInput,
    ParsedRequestContext, ProjectedLlmRequestHistory, RequestContextLimits,
    insert_content_attributes, llm_title, parse_json_request, project_request_content,
    project_tool_results, valid_model,
};
use crate::llm_pipeline::projection::ProjectionBatch;

pub(in crate::llm_pipeline::projection::projector) fn project_parsed_request(
    config: &SemanticRetentionConfig,
    context: &ParsedMessageContext,
    value: Value,
) -> Option<ProjectionBatch> {
    if !config.llm_layer_enabled() {
        return None;
    }
    let facts = LlmRequestFacts::from_json(
        &value,
        (config.l0_llm_call.context_compact.enabled || config.llm_trajectory_enabled()).then(
            || RequestContextLimits {
                observe_compaction: config.l0_llm_call.context_compact.enabled,
                max_metadata_bytes: config.l0_llm_call.context_compact.max_artifact_bytes as usize,
                max_reference_bytes: config.l0_llm_call.trajectory.max_structural_bytes_per_atom
                    as usize,
            },
        ),
    );
    let parsed = parse_json_request(&LlmRequestParserInput { facts: &facts })?;
    let tool_results = project_tool_results(
        config,
        context.identity.trace_id,
        &context.identity.process,
        context.start_at,
        &context.action_id,
        &value,
    );
    let mut body = LlmRequestBody {
        payload_bytes: context.payload_bytes,
        history_source_bytes: context.payload_bytes,
        json_valid: true,
        classifier_id: parsed.classifier_id.into(),
        protocol_id: parsed.protocol_id.map(str::to_owned),
        model: valid_model(parsed.model),
        json: Some(value),
        context: facts.context,
        content_available: true,
        background_kind: None,
    };
    let content =
        project_request_content(config, context.identity.trace_id, &context.action_id, &body)
            .ok()?;
    let mut attributes = context.attributes();
    attributes.insert(
        attrs::llm_request::PAYLOAD_BYTES.into(),
        context.payload_bytes.to_string(),
    );
    insert_content_attributes(config, &body, content.metadata, &mut attributes);
    let trajectory_history = content
        .trajectory_history
        .map(|history| ProjectedLlmRequestHistory {
            action_id: context.action_id.clone(),
            classifier_id: body.classifier_id,
            provider_context: body
                .context
                .as_mut()
                .map(ParsedRequestContext::take_provider_reference)
                .unwrap_or(super::ProviderContextReference::NotProvider),
            history,
        });
    let observation = if config.l0_llm_call.context_compact.enabled {
        body.json
            .take()
            .zip(body.context.take())
            .map(|(json, request_context)| LlmRequestObservation {
                action_id: context.action_id.clone(),
                history: request_context.history_source(json),
                context: request_context,
                history_source_bytes: body.history_source_bytes,
            })
    } else {
        None
    };
    Some(ProjectionBatch {
        actions: vec![SemanticAction {
            action_id: context.action_id.clone(),
            trace_id: context.identity.trace_id,
            process: context.identity.process,
            kind: SemanticActionKind::LlmRequest,
            title: llm_title(&attributes),
            start_time: context.start_at,
            end_time: Some(context.last_at),
            status: SemanticActionStatus::Success,
            completeness: SemanticActionCompleteness::Complete,
            attributes,
            evidence: context.evidence.clone(),
        }],
        llm_request_contents: content.content.into_iter().collect(),
        request_observations: observation.into_iter().collect(),
        llm_request_histories: trajectory_history.into_iter().collect(),
        llm_tool_results: tool_results,
        ..ProjectionBatch::default()
    })
}
