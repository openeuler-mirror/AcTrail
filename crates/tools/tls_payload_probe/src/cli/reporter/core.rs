//! Human-readable capture stream formatting.

use std::fmt::Write as FmtWrite;
use std::process::ExitStatus;

use crate::ToolResult;
use crate::capture::{
    AssembledHttp, CaptureEvent, HttpBody, HttpBodyFragment, HttpBodyFragmentBody, SseFrame,
};
use crate::cli::format::{Style, flags, normalize_text, redact_text, write_text_block};
use crate::cli::output::Output;
use crate::cli::report_config::{EventFilter, RedactionMode, ReporterConfig};
use crate::llm_projection::LlmOutput;

pub(crate) struct Reporter {
    sequence: usize,
    http_sequence: usize,
    fragment_sequence: usize,
    sse_sequence: usize,
    pub(super) llm_request_sequence: usize,
    pub(super) llm_delta_sequence: usize,
    pub(super) llm_message_sequence: usize,
    pub(super) style: Style,
    redaction: RedactionMode,
    events: EventFilter,
}

impl Reporter {
    pub(crate) fn new(config: ReporterConfig) -> Self {
        Self {
            sequence: 0,
            http_sequence: 0,
            fragment_sequence: 0,
            sse_sequence: 0,
            llm_request_sequence: 0,
            llm_delta_sequence: 0,
            llm_message_sequence: 0,
            style: Style::auto(),
            redaction: config.redaction,
            events: config.events,
        }
    }

    pub(crate) fn event(&mut self, event: &CaptureEvent) -> ToolResult<()> {
        if !self.events.payload() {
            return Ok(());
        }
        self.sequence += 1;
        let mut output = String::new();
        let _ = writeln!(
            output,
            "{}: {} {}={} {}={} pid={} tid={} stream=0x{:x} requested={} captured={} {}={}",
            self.style.payload_label(self.sequence),
            self.style.direction(event.direction),
            self.style.key("provider"),
            self.style.provider(&event.provider),
            self.style.key("symbol"),
            self.style.symbol(&event.symbol),
            event.pid,
            event.tid,
            event.stream_key,
            event.requested_size,
            event.captured.len(),
            self.style.key("flags"),
            flags(event, self.style)
        );
        Output::stdout(&output)
    }

    pub(crate) fn http_message(&mut self, message: &AssembledHttp) -> ToolResult<()> {
        if !self.events.http() {
            return Ok(());
        }
        self.http_sequence += 1;
        let mut output = String::new();
        let _ = writeln!(
            output,
            "{}: {} pid={} stream=0x{:x} {}",
            self.style.http_label(self.http_sequence),
            self.style.direction(message.direction),
            message.pid,
            message.stream_key,
            self.style.marker(&message.first_line)
        );
        let _ = writeln!(output, "  {}:", self.style.key("headers"));
        for header in &message.headers {
            let line = format!("{}: {}", header.name, header.value);
            let _ = writeln!(output, "    - {}", self.display_text(&line));
        }
        self.write_body(&mut output, message);
        Output::stdout(&output)
    }

    pub(crate) fn http_body_fragment(&mut self, fragment: &HttpBodyFragment) -> ToolResult<()> {
        if !self.events.http() {
            return Ok(());
        }
        self.fragment_sequence += 1;
        let mut output = String::new();
        let _ = write!(
            output,
            "{}: {} pid={} stream=0x{:x} {}",
            self.style.fragment_label(self.fragment_sequence),
            self.style.direction(fragment.direction),
            fragment.pid,
            fragment.stream_key,
            self.style.marker(&fragment.first_line)
        );
        match &fragment.body {
            HttpBodyFragmentBody::Text { bytes, text, .. } => {
                if fragment.is_event_stream() {
                    let _ = writeln!(
                        output,
                        " event_stream_text bytes={} frames=reported_separately",
                        bytes
                    );
                } else {
                    let _ = writeln!(output, " text bytes={bytes}");
                    write_text_block(&mut output, &self.display_text(text));
                }
            }
            HttpBodyFragmentBody::Binary { bytes, .. } => {
                let _ = writeln!(output, " binary bytes={bytes}");
            }
        }
        Output::stdout(&output)
    }

    pub(crate) fn llm_output(&mut self, output: &LlmOutput) -> ToolResult<()> {
        if !self.events.llm() {
            return Ok(());
        }
        match output {
            LlmOutput::Request(request) => self.llm_request(request),
            LlmOutput::Delta(delta) => self.llm_delta(delta),
            LlmOutput::Message(message) => self.llm_message(message),
        }
    }

    pub(crate) fn target_exit(&mut self, status: ExitStatus) -> ToolResult<()> {
        if !self.events.target() {
            return Ok(());
        }
        let mut output = String::new();
        let _ = writeln!(output, "{}: {status}", self.style.exit_label());
        Output::stdout(&output)
    }
}

impl Reporter {
    fn write_body(&self, output: &mut String, message: &AssembledHttp) {
        if message.is_event_stream() {
            self.write_event_stream_body(output, &message.body);
            return;
        }
        let body = &message.body;
        match body {
            HttpBody::Empty => {
                let _ = writeln!(output, "  {}: empty", self.style.key("body"));
            }
            HttpBody::Text { bytes, text } => {
                let _ = writeln!(output, "  {}: text bytes={bytes}", self.style.key("body"));
                write_text_block(output, &self.display_text(text));
            }
            HttpBody::Binary { bytes } => {
                let _ = writeln!(output, "  {}: binary bytes={bytes}", self.style.key("body"));
            }
            HttpBody::DecodedText {
                encoding,
                compressed_bytes,
                decoded_bytes,
                text,
            } => {
                let _ = writeln!(
                    output,
                    "  {}: decoded_text encoding={} compressed={} decoded={}",
                    self.style.key("body"),
                    self.style.marker(encoding),
                    compressed_bytes,
                    decoded_bytes
                );
                write_text_block(output, &self.display_text(text));
            }
            HttpBody::DecodedBinary {
                encoding,
                compressed_bytes,
                decoded_bytes,
            } => {
                let _ = writeln!(
                    output,
                    "  {}: decoded_binary encoding={} compressed={} decoded={}",
                    self.style.key("body"),
                    self.style.marker(encoding),
                    compressed_bytes,
                    decoded_bytes
                );
            }
            HttpBody::DecodeSkipped {
                encoding,
                compressed_bytes,
                limit_bytes,
            } => {
                let _ = writeln!(
                    output,
                    "  {}: decode_skipped encoding={} compressed={} limit={}",
                    self.style.key("body"),
                    self.style.warning(encoding),
                    compressed_bytes,
                    limit_bytes
                );
            }
            HttpBody::DecodeFailed {
                encoding,
                compressed_bytes,
                error,
            } => {
                let _ = writeln!(
                    output,
                    "  {}: decode_failed encoding={} compressed={} error={}",
                    self.style.key("body"),
                    self.style.warning(encoding),
                    compressed_bytes,
                    error
                );
            }
            HttpBody::Partial {
                buffered_bytes,
                reason,
            } => {
                let _ = writeln!(
                    output,
                    "  {}: partial buffered={} reason={}",
                    self.style.key("body"),
                    buffered_bytes,
                    reason
                );
            }
            HttpBody::PartialText {
                bytes,
                buffered_bytes,
                reason,
                text,
            } => {
                let _ = writeln!(
                    output,
                    "  {}: partial_text bytes={} buffered={} reason={}",
                    self.style.key("body"),
                    bytes,
                    buffered_bytes,
                    reason
                );
                write_text_block(output, &self.display_text(text));
            }
            HttpBody::PartialDecodedText {
                encoding,
                compressed_bytes,
                decoded_bytes,
                buffered_bytes,
                reason,
                text,
            } => {
                let _ = writeln!(
                    output,
                    "  {}: partial_decoded_text encoding={} compressed={} decoded={} buffered={} reason={}",
                    self.style.key("body"),
                    self.style.marker(encoding),
                    compressed_bytes,
                    decoded_bytes,
                    buffered_bytes,
                    reason
                );
                write_text_block(output, &self.display_text(text));
            }
            HttpBody::Streamed { bytes } => {
                let _ = writeln!(
                    output,
                    "  {}: streamed bytes={} fragments=reported_separately",
                    self.style.key("body"),
                    bytes
                );
            }
        }
    }

    fn write_event_stream_body(&self, output: &mut String, body: &HttpBody) {
        match body {
            HttpBody::Empty => {
                let _ = writeln!(output, "  {}: empty", self.style.key("body"));
            }
            HttpBody::Text { bytes, .. } => {
                let _ = writeln!(
                    output,
                    "  {}: event_stream_text bytes={} frames=reported_separately",
                    self.style.key("body"),
                    bytes
                );
            }
            HttpBody::DecodedText {
                encoding,
                compressed_bytes,
                decoded_bytes,
                ..
            } => {
                let _ = writeln!(
                    output,
                    "  {}: decoded_event_stream_text encoding={} compressed={} decoded={} frames=reported_separately",
                    self.style.key("body"),
                    self.style.marker(encoding),
                    compressed_bytes,
                    decoded_bytes
                );
            }
            HttpBody::PartialText {
                bytes,
                buffered_bytes,
                reason,
                ..
            } => {
                let _ = writeln!(
                    output,
                    "  {}: partial_event_stream_text bytes={} buffered={} reason={} frames=reported_separately",
                    self.style.key("body"),
                    bytes,
                    buffered_bytes,
                    reason
                );
            }
            HttpBody::PartialDecodedText {
                encoding,
                compressed_bytes,
                decoded_bytes,
                buffered_bytes,
                reason,
                ..
            } => {
                let _ = writeln!(
                    output,
                    "  {}: partial_decoded_event_stream_text encoding={} compressed={} decoded={} buffered={} reason={} frames=reported_separately",
                    self.style.key("body"),
                    self.style.marker(encoding),
                    compressed_bytes,
                    decoded_bytes,
                    buffered_bytes,
                    reason
                );
            }
            HttpBody::Streamed { bytes } => {
                let _ = writeln!(
                    output,
                    "  {}: streamed_event_stream bytes={} frames=reported_separately",
                    self.style.key("body"),
                    bytes
                );
            }
            HttpBody::Binary { bytes } => {
                let _ = writeln!(
                    output,
                    "  {}: event_stream_binary bytes={}",
                    self.style.key("body"),
                    bytes
                );
            }
            HttpBody::DecodedBinary {
                encoding,
                compressed_bytes,
                decoded_bytes,
            } => {
                let _ = writeln!(
                    output,
                    "  {}: decoded_event_stream_binary encoding={} compressed={} decoded={}",
                    self.style.key("body"),
                    self.style.marker(encoding),
                    compressed_bytes,
                    decoded_bytes
                );
            }
            HttpBody::DecodeSkipped {
                encoding,
                compressed_bytes,
                limit_bytes,
            } => {
                let _ = writeln!(
                    output,
                    "  {}: event_stream_decode_skipped encoding={} compressed={} limit={}",
                    self.style.key("body"),
                    self.style.warning(encoding),
                    compressed_bytes,
                    limit_bytes
                );
            }
            HttpBody::DecodeFailed {
                encoding,
                compressed_bytes,
                error,
            } => {
                let _ = writeln!(
                    output,
                    "  {}: event_stream_decode_failed encoding={} compressed={} error={}",
                    self.style.key("body"),
                    self.style.warning(encoding),
                    compressed_bytes,
                    error
                );
            }
            HttpBody::Partial {
                buffered_bytes,
                reason,
            } => {
                let _ = writeln!(
                    output,
                    "  {}: partial_event_stream buffered={} reason={}",
                    self.style.key("body"),
                    buffered_bytes,
                    reason
                );
            }
        }
    }

    pub(crate) fn sse_frame(&mut self, frame: &SseFrame) -> ToolResult<()> {
        if !self.events.sse() {
            return Ok(());
        }
        self.sse_sequence += 1;
        let event = frame.event.as_deref().unwrap_or("message");
        let status = if frame.complete {
            "complete"
        } else {
            "partial"
        };
        let mut output = String::new();
        let _ = writeln!(
            output,
            "{}: {} pid={} stream=0x{:x} {}={} data_bytes={} {}={}",
            self.style.sse_label(self.sse_sequence),
            self.style.direction(frame.direction),
            frame.pid,
            frame.stream_key,
            self.style.key("event"),
            self.style.marker(event),
            frame.data_bytes,
            self.style.key("status"),
            self.style.marker(status)
        );
        Output::stdout(&output)
    }

    pub(super) fn display_text(&self, text: &str) -> String {
        let normalized = normalize_text(text);
        match self.redaction {
            RedactionMode::Redact => redact_text(&normalized),
            RedactionMode::None => normalized,
        }
    }
}
