//! Human-readable WebSocket message formatting.

use std::fmt::Write as FmtWrite;

use crate::ToolResult;
use crate::capture::{WebSocketMessage, WebSocketPayload};
use crate::cli::format::write_text_block;
use crate::cli::output::Output;

use super::core::Reporter;

impl Reporter {
    pub(crate) fn websocket_message(&mut self, message: &WebSocketMessage) -> ToolResult<()> {
        if !self.events.websocket() {
            return Ok(());
        }
        self.websocket_sequence += 1;
        let mut output = String::new();
        let _ = writeln!(
            output,
            "{}: {} pid={} stream=0x{:x} path={} compressed={} wire_bytes={} payload_bytes={}",
            self.style.websocket_label(self.websocket_sequence),
            self.style.direction(message.direction),
            message.pid,
            message.stream_key,
            self.style.marker(&message.path),
            message.compressed,
            message.wire_bytes,
            message.payload_bytes()
        );
        match &message.payload {
            WebSocketPayload::Text(text) => {
                write_text_block(&mut output, &self.display_text(text));
            }
            WebSocketPayload::Binary(bytes) => {
                let _ = writeln!(output, "<binary bytes={}>", bytes.len());
            }
        }
        Output::stdout(&output)
    }
}
