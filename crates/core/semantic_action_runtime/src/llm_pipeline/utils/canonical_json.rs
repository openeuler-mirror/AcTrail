use std::io::{self, Write as _};

use serde_json::Value;

pub(in crate::llm_pipeline) struct CanonicalJsonWriter {
    output: Vec<u8>,
    max_bytes: u64,
}

#[derive(Clone, Copy)]
enum JsonScope {
    Value,
    HistoryMessage,
    HistoryContent,
    ContentBlock,
}

impl CanonicalJsonWriter {
    pub(in crate::llm_pipeline) fn new(max_bytes: u64) -> Self {
        Self {
            output: Vec::new(),
            max_bytes,
        }
    }

    pub(in crate::llm_pipeline) fn serialize_bytes(mut self, value: &Value) -> io::Result<Vec<u8>> {
        self.write_value(value)?;
        Ok(self.output)
    }

    pub(in crate::llm_pipeline) fn serialize(self, value: &Value) -> io::Result<String> {
        Ok(String::from_utf8(self.serialize_bytes(value)?)
            .expect("canonical JSON serialization emits valid UTF-8"))
    }

    pub(in crate::llm_pipeline) fn serialize_history_content(
        self,
        value: &Value,
    ) -> io::Result<String> {
        self.serialize_history(value, JsonScope::HistoryContent)
    }

    pub(in crate::llm_pipeline) fn serialize_history_message(
        self,
        value: &Value,
    ) -> io::Result<String> {
        self.serialize_history(value, JsonScope::HistoryMessage)
    }

    pub(in crate::llm_pipeline) fn is_content_cache_metadata(key: &str) -> bool {
        key == "cache_control"
    }

    fn serialize_history(mut self, value: &Value, scope: JsonScope) -> io::Result<String> {
        self.write_scoped_value(value, scope)?;
        Ok(String::from_utf8(self.output).expect("canonical JSON serialization emits valid UTF-8"))
    }

    fn write_value(&mut self, value: &Value) -> io::Result<()> {
        self.write_scoped_value(value, JsonScope::Value)
    }

    fn write_scoped_value(&mut self, value: &Value, scope: JsonScope) -> io::Result<()> {
        match value {
            Value::Null => self.write_all(b"null"),
            Value::Bool(value) => self.write_all(if *value { b"true" } else { b"false" }),
            Value::Number(value) => self.write_all(value.to_string().as_bytes()),
            Value::String(value) => self.write_string(value),
            Value::Array(values) => {
                self.require_remaining((values.len() as u64).saturating_mul(2).saturating_add(1))?;
                self.write_all(b"[")?;
                let child_scope = match scope {
                    JsonScope::HistoryContent => JsonScope::ContentBlock,
                    _ => JsonScope::Value,
                };
                for (index, value) in values.iter().enumerate() {
                    if index > 0 {
                        self.write_all(b",")?;
                    }
                    self.write_scoped_value(value, child_scope)?;
                }
                self.write_all(b"]")
            }
            Value::Object(object) => {
                let content_block =
                    matches!(scope, JsonScope::HistoryContent | JsonScope::ContentBlock);
                let retained_count = object.len()
                    - usize::from(content_block && object.contains_key("cache_control"));
                self.require_remaining(
                    (retained_count as u64).saturating_mul(5).saturating_add(1),
                )?;
                self.write_all(b"{")?;
                let mut keys = Vec::with_capacity(retained_count);
                keys.extend(
                    object
                        .keys()
                        .filter(|key| !content_block || !Self::is_content_cache_metadata(key)),
                );
                keys.sort_unstable();
                for (index, key) in keys.into_iter().enumerate() {
                    if index > 0 {
                        self.write_all(b",")?;
                    }
                    self.write_string(key)?;
                    self.write_all(b":")?;
                    // Only protocol content envelopes exclude cache metadata.
                    // Tool inputs, arguments and other nested values retain every field.
                    let child_scope = match scope {
                        JsonScope::HistoryMessage if key == "content" => JsonScope::HistoryContent,
                        _ => JsonScope::Value,
                    };
                    self.write_scoped_value(&object[key], child_scope)?;
                }
                self.write_all(b"}")
            }
        }
    }

    fn write_string(&mut self, value: &str) -> io::Result<()> {
        // Reject oversized raw strings before serde scans them for escaping.
        self.require_remaining((value.len() as u64).saturating_add(2))?;
        serde_json::to_writer(self, value).map_err(io::Error::other)
    }

    fn require_remaining(&self, bytes: u64) -> io::Result<()> {
        if bytes > self.max_bytes.saturating_sub(self.output.len() as u64) {
            return Err(io::Error::other(
                "canonical JSON exceeds configured byte limit",
            ));
        }
        Ok(())
    }
}

impl io::Write for CanonicalJsonWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.require_remaining(bytes.len() as u64)?;
        self.output.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
