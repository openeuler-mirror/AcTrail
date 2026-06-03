//! Shared CLI formatting helpers.

use std::fmt::Write as FmtWrite;

use crate::capture::{CaptureDirection, CaptureEvent};
use crate::cli::output::Output;

const ANSI_RESET: &str = "\x1b[0m";
const ANSI_BOLD_CYAN: &str = "\x1b[1;36m";
const ANSI_BOLD_BLUE: &str = "\x1b[1;34m";
const ANSI_BOLD_GREEN: &str = "\x1b[1;32m";
const ANSI_BOLD_MAGENTA: &str = "\x1b[1;35m";
const ANSI_BOLD_YELLOW: &str = "\x1b[1;33m";
const ANSI_BOLD_RED: &str = "\x1b[1;31m";
const ANSI_DIM: &str = "\x1b[2m";

#[derive(Clone, Copy)]
pub(super) struct Style {
    enabled: bool,
}

impl Style {
    pub(super) fn auto() -> Self {
        Self {
            enabled: Output::stdout_supports_color(),
        }
    }

    pub(super) fn payload_label(self, sequence: usize) -> String {
        self.paint(ANSI_BOLD_CYAN, &format!("payload_event #{sequence}"))
    }

    pub(super) fn http_label(self, sequence: usize) -> String {
        self.paint(ANSI_BOLD_CYAN, &format!("http_message #{sequence}"))
    }

    pub(super) fn fragment_label(self, sequence: usize) -> String {
        self.paint(ANSI_BOLD_CYAN, &format!("http_body_fragment #{sequence}"))
    }

    pub(super) fn sse_label(self, sequence: usize) -> String {
        self.paint(ANSI_BOLD_MAGENTA, &format!("sse_frame #{sequence}"))
    }

    pub(super) fn llm_delta_label(self, sequence: usize) -> String {
        self.paint(ANSI_BOLD_GREEN, &format!("llm_delta #{sequence}"))
    }

    pub(super) fn llm_request_label(self, sequence: usize) -> String {
        self.paint(ANSI_BOLD_GREEN, &format!("llm_request #{sequence}"))
    }

    pub(super) fn llm_message_label(self, sequence: usize) -> String {
        self.paint(ANSI_BOLD_GREEN, &format!("llm_message #{sequence}"))
    }

    pub(super) fn exit_label(self) -> String {
        self.paint(ANSI_BOLD_CYAN, "target_exit")
    }

    pub(super) fn ring_stats_label(self) -> String {
        self.paint(ANSI_BOLD_CYAN, "ring_stats")
    }

    pub(super) fn direction(self, direction: CaptureDirection) -> String {
        match direction {
            CaptureDirection::Inbound => self.paint(ANSI_BOLD_BLUE, direction.as_str()),
            CaptureDirection::Outbound => self.paint(ANSI_BOLD_GREEN, direction.as_str()),
        }
    }

    pub(super) fn key(self, key: &str) -> String {
        self.paint(ANSI_DIM, key)
    }

    pub(super) fn provider(self, provider: &str) -> String {
        self.paint(ANSI_BOLD_MAGENTA, provider)
    }

    pub(super) fn symbol(self, symbol: &str) -> String {
        self.paint(ANSI_BOLD_CYAN, symbol)
    }

    pub(super) fn warning(self, value: &str) -> String {
        self.paint(ANSI_BOLD_RED, value)
    }

    pub(super) fn marker(self, value: &str) -> String {
        self.paint(ANSI_BOLD_YELLOW, value)
    }

    fn paint(self, color: &str, value: &str) -> String {
        if self.enabled {
            format!("{color}{value}{ANSI_RESET}")
        } else {
            value.to_string()
        }
    }
}

pub(super) fn flags(event: &CaptureEvent, style: Style) -> String {
    let mut flags = Vec::new();
    if event.flags.truncated {
        flags.push(style.warning("truncated"));
    }
    if event.flags.rustls_chunk {
        flags.push(style.marker("rustls_chunk"));
    }
    if flags.is_empty() {
        "none".to_string()
    } else {
        flags.join(",")
    }
}

pub(super) fn write_text_block(output: &mut String, text: &str) {
    if text.is_empty() {
        let _ = writeln!(output);
        return;
    }
    for line in text.lines() {
        let _ = writeln!(output, "{}", line);
    }
    if text.ends_with('\n') {
        let _ = writeln!(output);
    }
}

pub(super) fn normalize_text(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

pub(super) fn redact_text(text: &str) -> String {
    text.lines().map(redact_line).collect::<Vec<_>>().join("\n")
}

fn redact_line(line: &str) -> String {
    let lower = line.to_ascii_lowercase();
    if lower.starts_with("authorization:") {
        return "authorization: <redacted>".to_string();
    }
    if lower.starts_with("cookie:") {
        return "cookie: <redacted>".to_string();
    }
    redact_bearer(line)
}

fn redact_bearer(line: &str) -> String {
    let lower = line.to_ascii_lowercase();
    let Some(index) = lower.find("bearer ") else {
        return line.to_string();
    };
    let mut redacted = String::new();
    redacted.push_str(&line[..index + "bearer ".len()]);
    redacted.push_str("<redacted>");
    redacted
}
