//! Prompt fingerprints used to correlate an invocation with its child request.

use std::collections::BTreeSet;

use serde_json::json;

use crate::llm_pipeline::canonical_llm_json;

#[derive(Default)]
pub(super) struct PromptFingerprint {
    message_hashes: BTreeSet<String>,
    preview: Option<String>,
}

impl PromptFingerprint {
    pub(super) fn from_prompt(prompt: &str) -> Self {
        let message_hashes = [
            json!({"role": "user", "content": prompt}),
            json!({"role": "human", "content": prompt}),
            json!({"role": "user", "content": [{"type": "text", "text": prompt}]}),
            json!({"role": "user", "content": [{"type": "input_text", "text": prompt}]}),
            json!({"type": "message", "role": "user", "content": [{"type": "input_text", "text": prompt}]}),
        ]
        .into_iter()
        .map(|message| canonical_llm_json(&message).1)
        .collect();
        Self {
            message_hashes,
            preview: Some(Self::preview(prompt)),
        }
    }

    pub(super) fn into_parts(self) -> (BTreeSet<String>, Option<String>) {
        (self.message_hashes, self.preview)
    }

    fn preview(prompt: &str) -> String {
        let mut preview = String::new();
        for (index, ch) in prompt.trim().chars().enumerate() {
            if index >= 160 {
                preview.push_str("...");
                break;
            }
            preview.push(ch);
        }
        preview
    }
}
