use serde_json::Value;

#[derive(Default)]
pub(in crate::llm_pipeline) struct ResponseTexts {
    pub(in crate::llm_pipeline) content_text: Option<String>,
    pub(in crate::llm_pipeline) reasoning_text: Option<String>,
    pub(in crate::llm_pipeline) tool_observed: bool,
    content_observed: bool,
    reasoning_observed: bool,
}

impl ResponseTexts {
    pub(in crate::llm_pipeline) fn extract(value: &Value, retain: bool) -> Self {
        let mut texts = Self::default();
        texts.observe(value, retain);
        texts
    }

    pub(in crate::llm_pipeline) fn chunk_count(&self) -> usize {
        usize::from(self.content_observed) + usize::from(self.reasoning_observed)
    }

    fn observe(&mut self, value: &Value, retain: bool) {
        match value {
            Value::Array(items) => {
                for item in items {
                    self.observe(item, retain);
                }
            }
            Value::Object(object) => {
                for key in ["content", "text", "output_text"] {
                    if let Some(text) = object
                        .get(key)
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                    {
                        self.content_observed = true;
                        if retain {
                            self.content_text
                                .get_or_insert_with(String::new)
                                .push_str(text);
                        }
                    }
                }
                for key in ["reasoning_content", "thinking"] {
                    if let Some(text) = object
                        .get(key)
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                    {
                        self.reasoning_observed = true;
                        if retain {
                            self.reasoning_text
                                .get_or_insert_with(String::new)
                                .push_str(text);
                        }
                    }
                }
                self.tool_observed |= matches!(
                    object.get("type").and_then(Value::as_str),
                    Some("function_call" | "custom_tool_call")
                ) && object
                    .get("call_id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| !id.is_empty());
                self.tool_observed |= object
                    .get("tool_calls")
                    .and_then(Value::as_array)
                    .is_some_and(|calls| {
                        calls.iter().any(|call| {
                            call.get("index").and_then(Value::as_u64).is_some()
                                || call
                                    .get("id")
                                    .and_then(Value::as_str)
                                    .is_some_and(|id| !id.is_empty())
                        })
                    });
                // Code-mode providers can omit call_id on a custom exec item.
                // Its typed declaration is protocol evidence even when the host
                // does not retain or interpret the code's tool arguments.
                self.tool_observed |= object.get("type").and_then(Value::as_str)
                    == Some("custom_tool_call")
                    && object.get("name").and_then(Value::as_str) == Some("exec")
                    && ["input", "arguments", "code"]
                        .iter()
                        .any(|key| object.contains_key(*key));
                for key in ["content", "message", "delta", "choices", "output"] {
                    if let Some(child) = object.get(key) {
                        self.observe(child, retain);
                    }
                }
            }
            _ => {}
        }
    }
}
