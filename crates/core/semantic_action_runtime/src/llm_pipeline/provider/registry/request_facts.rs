use serde_json::Value;

/// Classification facts for one request, independent of retained content.
#[derive(Default)]
pub(in crate::llm_pipeline) struct LlmRequestFacts {
    pub(in crate::llm_pipeline) model_present: bool,
    pub(in crate::llm_pipeline) model: Option<String>,
    pub(in crate::llm_pipeline) model_name_present: bool,
    pub(in crate::llm_pipeline) model_name: Option<String>,
    pub(in crate::llm_pipeline) provider_model_name: Option<String>,
    pub(in crate::llm_pipeline) input_present: bool,
    pub(in crate::llm_pipeline) messages_have_text: bool,
    pub(in crate::llm_pipeline) context_signal: bool,
}

impl LlmRequestFacts {
    pub(in crate::llm_pipeline) fn from_json(value: &Value) -> Self {
        let Some(object) = value.as_object() else {
            return Self::default();
        };
        Self {
            model_present: object.contains_key("model"),
            model: object
                .get("model")
                .and_then(Value::as_str)
                .map(str::to_owned),
            model_name_present: object.contains_key("model_name"),
            model_name: object
                .get("model_name")
                .and_then(Value::as_str)
                .map(str::to_owned),
            provider_model_name: object
                .get("provider_model_name")
                .and_then(Value::as_str)
                .map(str::to_owned),
            input_present: ["messages", "prompt", "input"]
                .iter()
                .any(|key| object.contains_key(*key)),
            messages_have_text: object
                .get("messages")
                .and_then(Value::as_array)
                .is_some_and(|messages| messages.iter().any(Self::message_has_text)),
            context_signal: object.contains_key("tools")
                || ["user_input", "session_id", "conversation_id", "config_name"]
                    .iter()
                    .any(|key| {
                        object
                            .get(*key)
                            .and_then(Value::as_str)
                            .is_some_and(|s| !s.is_empty())
                    }),
        }
    }

    pub(in crate::llm_pipeline) fn generic_match(&self) -> bool {
        self.model_present && self.input_present
    }

    pub(in crate::llm_pipeline) fn structured_match(&self) -> bool {
        self.messages_have_text
            && (self.model.as_deref().is_some_and(|s| !s.is_empty())
                || (self.named_model().is_some() && self.context_signal))
    }

    pub(in crate::llm_pipeline) fn structured_model(&self) -> Option<&str> {
        self.named_model()
            .or_else(|| self.model.as_deref().filter(|s| !s.is_empty()))
    }

    fn named_model(&self) -> Option<&str> {
        if self.model_name_present {
            self.model_name.as_deref()
        } else {
            self.provider_model_name.as_deref()
        }
        .filter(|s| !s.is_empty())
    }

    fn message_has_text(value: &Value) -> bool {
        value.get("role").and_then(Value::as_str).is_some()
            && value.get("content").is_some_and(|content| match content {
                Value::String(text) => !text.is_empty(),
                Value::Array(items) => items.iter().any(|item| {
                    item.get("type").and_then(Value::as_str) == Some("text")
                        && item
                            .get("text")
                            .and_then(Value::as_str)
                            .is_some_and(|s| !s.is_empty())
                }),
                _ => false,
            })
    }
}
