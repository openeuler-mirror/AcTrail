use serde_json::Value;

#[derive(Clone, Copy)]
pub(in crate::llm_pipeline) struct RequestContextLimits {
    pub(in crate::llm_pipeline) observe_compaction: bool,
    pub(in crate::llm_pipeline) max_metadata_bytes: usize,
    pub(in crate::llm_pipeline) max_reference_bytes: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::llm_pipeline) struct RequestContextShape {
    pub(in crate::llm_pipeline) item_count: usize,
    pub(in crate::llm_pipeline) item_unit: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ProviderContextReference {
    NotProvider,
    Root,
    PreviousResponse(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::llm_pipeline) struct ExplicitCompactionIntent {
    pub(in crate::llm_pipeline) source: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::llm_pipeline) struct OpaqueContextArtifact {
    pub(in crate::llm_pipeline) item_count: usize,
    pub(in crate::llm_pipeline) item_unit: &'static str,
    pub(in crate::llm_pipeline) artifact_path: String,
    pub(in crate::llm_pipeline) artifact_bytes: usize,
    pub(in crate::llm_pipeline) has_following_user: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::llm_pipeline) struct ParsedRequestContext {
    pub(in crate::llm_pipeline) shape: Option<RequestContextShape>,
    pub(in crate::llm_pipeline) provider_reference: ProviderContextReference,
    pub(in crate::llm_pipeline) explicit_compaction: Option<ExplicitCompactionIntent>,
    pub(in crate::llm_pipeline) uses_previous_response: bool,
    pub(in crate::llm_pipeline) input_item_count: Option<usize>,
}

pub(super) struct RequestContextParser {
    limits: RequestContextLimits,
}

impl RequestContextParser {
    pub(super) fn new(limits: RequestContextLimits) -> Self {
        Self { limits }
    }

    pub(super) fn parse(&self, body: &Value) -> ParsedRequestContext {
        let provider_reference = self.provider_reference(body);
        if !self.limits.observe_compaction {
            return ParsedRequestContext {
                shape: None,
                provider_reference,
                explicit_compaction: None,
                uses_previous_response: false,
                input_item_count: None,
            };
        }
        let input = body.get("input").and_then(Value::as_array);
        let shape = body
            .get("messages")
            .and_then(Value::as_array)
            .map(|items| RequestContextShape {
                item_count: items.len(),
                item_unit: "message",
            })
            .or_else(|| {
                input.map(|items| RequestContextShape {
                    item_count: items.len(),
                    item_unit: "input_item",
                })
            });
        let uses_previous_response = body
            .get("previous_response_id")
            .is_some_and(|value| !value.is_null());
        ParsedRequestContext {
            shape,
            provider_reference,
            explicit_compaction: self.explicit_compaction(body),
            uses_previous_response,
            input_item_count: input.map(Vec::len),
        }
    }

    fn provider_reference(&self, body: &Value) -> ProviderContextReference {
        match body.get("previous_response_id") {
            None => ProviderContextReference::NotProvider,
            Some(Value::Null) => ProviderContextReference::Root,
            Some(Value::String(response_id))
                if !response_id.is_empty()
                    && response_id.len() <= self.limits.max_reference_bytes =>
            {
                ProviderContextReference::PreviousResponse(response_id.clone())
            }
            Some(_) => ProviderContextReference::Root,
        }
    }

    fn explicit_compaction(&self, body: &Value) -> Option<ExplicitCompactionIntent> {
        let trigger = body
            .get("input")
            .and_then(Value::as_array)?
            .first()
            .filter(|item| {
                item.get("type").and_then(Value::as_str) == Some("compaction_trigger")
            })?;
        let embedded = body
            .get("client_metadata")
            .and_then(|metadata| metadata.get("x-codex-turn-metadata"))
            .and_then(Value::as_str)
            .filter(|metadata| metadata.len() <= self.limits.max_metadata_bytes)
            .and_then(|metadata| serde_json::from_str::<Value>(metadata).ok());
        let request_kind = body
            .get("request_kind")
            .and_then(Value::as_str)
            .or_else(|| {
                body.get("metadata")
                    .and_then(|metadata| metadata.get("request_kind"))
                    .and_then(Value::as_str)
            })
            .or_else(|| {
                embedded
                    .as_ref()
                    .and_then(|metadata| metadata.get("request_kind"))
                    .and_then(Value::as_str)
            });
        if request_kind != Some("compaction") {
            return None;
        }
        let source = trigger
            .get("reason")
            .and_then(Value::as_str)
            .or_else(|| body.get("reason").and_then(Value::as_str))
            .or_else(|| {
                body.get("metadata")
                    .and_then(|metadata| metadata.get("reason"))
                    .and_then(Value::as_str)
            })
            .or_else(|| {
                embedded
                    .as_ref()
                    .and_then(|metadata| metadata.get("compaction"))
                    .and_then(|compaction| compaction.get("reason"))
                    .and_then(Value::as_str)
            });
        let trigger_kind = trigger
            .get("trigger")
            .and_then(Value::as_str)
            .or_else(|| body.get("trigger").and_then(Value::as_str))
            .or_else(|| {
                embedded
                    .as_ref()
                    .and_then(|metadata| metadata.get("compaction"))
                    .and_then(|compaction| compaction.get("trigger"))
                    .and_then(Value::as_str)
            });
        Some(ExplicitCompactionIntent {
            source: match (source, trigger_kind) {
                (Some("user_requested"), _) => "user_requested",
                (_, Some("manual")) => "manual",
                _ => "explicit_protocol",
            },
        })
    }
}

impl ParsedRequestContext {
    pub(in crate::llm_pipeline) fn take_provider_reference(&mut self) -> ProviderContextReference {
        std::mem::replace(
            &mut self.provider_reference,
            ProviderContextReference::NotProvider,
        )
    }

    /// Both built-in request parsers project this JSON context. Ownership moves
    /// once after retention projection; candidates share the immutable reader.
    pub(in crate::llm_pipeline) fn history_source(
        &self,
        body: Value,
    ) -> std::sync::Arc<super::request_history::RequestHistorySource> {
        std::sync::Arc::new(super::request_history::RequestHistorySource::new(body))
    }
}
