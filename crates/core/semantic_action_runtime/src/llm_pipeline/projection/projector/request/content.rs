use config_core::daemon::{LlmRequestContentRetention, SemanticRetentionConfig};

use super::super::super::retention::{
    CanonicalJsonWriter, FORMAT_VERSION, background_request_kind, canonical_request_content,
    request_shape_metadata,
};
use super::{
    CanonicalBodyExport, LlmRequestBody, RequestContentMetadata, RequestContentProjection,
};

pub(super) fn project_request_content(
    config: &SemanticRetentionConfig,
    trace_id: model_core::ids::TraceId,
    action_id: &str,
    body: &LlmRequestBody,
) -> Result<RequestContentProjection, String> {
    if !config.llm_layer_enabled() {
        return Ok(RequestContentProjection {
            content: None,
            metadata: None,
            trajectory_history: None,
        });
    }
    if !body.content_available {
        return Ok(RequestContentProjection {
            content: None,
            metadata: None,
            trajectory_history: None,
        });
    }
    match config.l0_llm_call.request_content {
        LlmRequestContentRetention::None => Ok(RequestContentProjection {
            content: None,
            trajectory_history: None,
            metadata: Some(RequestContentMetadata {
                state: "none",
                format_version: None,
                body_export: None,
                block_count: None,
                message_preview: None,
                user_message_count: None,
                tool_result_count: None,
                background_kind: body
                    .background_kind
                    .or_else(|| body.json.as_ref().and_then(background_request_kind)),
            }),
        }),
        LlmRequestContentRetention::Shape => Ok(shape_projection(body)),
        LlmRequestContentRetention::CanonicalBlocks => {
            let Some(value) = body.json.as_ref() else {
                return Ok(shape_projection(body));
            };
            let content = canonical_request_content(
                trace_id,
                action_id,
                value,
                config.llm_trajectory_enabled(),
            )?;
            let canonical_body_export = config.llm_request_body_export_enabled().then(|| {
                match CanonicalJsonWriter::new(config.l0_llm_call.request_body_export_max_bytes)
                    .serialize(value)
                {
                    Ok(json) => CanonicalBodyExport::Exported(json),
                    Err(_) => CanonicalBodyExport::TooLarge,
                }
            });
            Ok(RequestContentProjection {
                metadata: Some(RequestContentMetadata {
                    state: "canonical_blocks",
                    format_version: Some(FORMAT_VERSION),
                    body_export: canonical_body_export,
                    block_count: Some(content.block_count),
                    message_preview: content.message_preview,
                    user_message_count: Some(content.user_message_count),
                    tool_result_count: Some(content.tool_result_count),
                    background_kind: content.background_kind,
                }),
                content: Some(content.write),
                trajectory_history: content.trajectory_history,
            })
        }
    }
}

fn shape_projection(body: &LlmRequestBody) -> RequestContentProjection {
    let (message_preview, user_messages, tool_result_count, background_kind) = body
        .json
        .as_ref()
        .map_or((None, None, None, None), |value| {
            let (preview, user_messages, tool_result_count, background_kind) =
                request_shape_metadata(value);
            (
                preview,
                Some(user_messages),
                Some(tool_result_count),
                background_kind,
            )
        });
    RequestContentProjection {
        content: None,
        trajectory_history: None,
        metadata: Some(RequestContentMetadata {
            state: "shape",
            format_version: body.json.as_ref().map(|_| FORMAT_VERSION),
            body_export: None,
            block_count: None,
            message_preview,
            user_message_count: user_messages.as_ref().map(|metadata| metadata.count),
            tool_result_count,
            background_kind,
        }),
    }
}
