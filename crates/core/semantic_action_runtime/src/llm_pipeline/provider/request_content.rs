use serde_json::Value;

/// Borrowed interpretation of a JSON request content block.
pub(in crate::llm_pipeline) struct RequestContentBlock<'a> {
    value: &'a Value,
}

impl<'a> RequestContentBlock<'a> {
    pub(in crate::llm_pipeline) fn new(value: &'a Value) -> Self {
        Self { value }
    }

    pub(in crate::llm_pipeline) fn text(&self) -> Option<&'a str> {
        self.value.as_str().or_else(|| {
            ["text", "content", "input_text", "output_text"]
                .iter()
                .find_map(|key| self.value.get(key).and_then(Value::as_str))
        })
    }

    /// Content-block results use the same rule for canonical history and
    /// identifying user messages containing only tool results.
    pub(in crate::llm_pipeline) fn is_tool_result(&self) -> bool {
        self.value
            .get("type")
            .and_then(Value::as_str)
            .is_some_and(|kind| matches!(kind, "tool_result" | "tool-result"))
    }
}
