use std::fmt::Write as FmtWrite;

use crate::ToolResult;
use crate::cli::format::write_text_block;
use crate::cli::output::Output;
use crate::llm_projection::{LlmDelta, LlmMessage, LlmRequest};

use super::core::Reporter;

impl Reporter {
    pub(super) fn llm_delta(&mut self, delta: &LlmDelta) -> ToolResult<()> {
        self.llm_delta_sequence += 1;
        let mut output = String::new();
        let _ = writeln!(
            output,
            "{}: {} pid={} stream=0x{:x} output_index={} content_index={}",
            self.style.llm_delta_label(self.llm_delta_sequence),
            self.style.direction(delta.direction),
            delta.pid,
            delta.stream_key,
            delta.output_index,
            delta.content_index
        );
        write_text_block(&mut output, &self.display_text(&delta.text));
        Output::stdout(&output)
    }

    pub(super) fn llm_request(&mut self, request: &LlmRequest) -> ToolResult<()> {
        self.llm_request_sequence += 1;
        let mut output = String::new();
        let _ = write!(
            output,
            "{}: {} pid={} stream=0x{:x} {}={}",
            self.style.llm_request_label(self.llm_request_sequence),
            self.style.direction(request.direction),
            request.pid,
            request.stream_key,
            self.style.key("schema"),
            self.style.marker(request.schema.as_str())
        );
        if let Some(model) = &request.model {
            let _ = write!(
                output,
                " {}={}",
                self.style.key("model"),
                self.style.marker(model)
            );
        }
        if let Some(stream) = request.stream {
            let _ = write!(output, " {}={stream}", self.style.key("stream"));
        }
        let _ = writeln!(output);
        for item in &request.items {
            let _ = writeln!(output, "  - {}:", self.style.key(&item.label));
            write_text_block(&mut output, &self.display_text(&item.text));
        }
        Output::stdout(&output)
    }

    pub(super) fn llm_message(&mut self, message: &LlmMessage) -> ToolResult<()> {
        self.llm_message_sequence += 1;
        let mut output = String::new();
        let _ = write!(
            output,
            "{}: {} pid={} stream=0x{:x} output_index={} content_index={} {}={}",
            self.style.llm_message_label(self.llm_message_sequence),
            self.style.direction(message.direction),
            message.pid,
            message.stream_key,
            message.output_index,
            message.content_index,
            self.style.key("status"),
            self.style.marker(message.status.as_str())
        );
        if let Some(reason) = &message.reason {
            let _ = write!(output, " {}={}", self.style.key("reason"), reason);
        }
        let _ = writeln!(output);
        write_text_block(&mut output, &self.display_text(&message.text));
        Output::stdout(&output)
    }
}
