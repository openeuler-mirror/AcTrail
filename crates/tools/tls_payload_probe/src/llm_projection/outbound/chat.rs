use serde_json::Value;

use crate::capture::{AssembledHttp, CaptureDirection};

use super::super::model::{LlmOutput, LlmRequestSchema, request};
use super::common::{message_items, model, stream};

const PATH_CHAT_COMPLETIONS: &str = "/chat/completions";

#[derive(Debug, Default)]
pub(in crate::llm_projection) struct ChatCompletionsRequestParser;

impl ChatCompletionsRequestParser {
    pub(in crate::llm_projection) fn parse(
        message: &AssembledHttp,
        value: &Value,
    ) -> Option<LlmOutput> {
        if message.direction != CaptureDirection::Outbound
            || !message.first_line.contains(PATH_CHAT_COMPLETIONS)
        {
            return None;
        }
        let items = message_items(value);
        if items.is_empty() {
            return None;
        }
        Some(request(
            message.pid,
            message.stream_key,
            message.direction,
            LlmRequestSchema::OpenAiChatCompletions,
            model(value),
            stream(value),
            items,
        ))
    }
}
