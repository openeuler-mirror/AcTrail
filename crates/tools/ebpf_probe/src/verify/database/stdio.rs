//! Stdio payload assertions for live verification.

use std::collections::HashSet;

use model_core::payload::{
    PayloadContentState, PayloadDirection, PayloadSegment, PayloadSourceBoundary,
};

pub(super) fn observed(segments: &[PayloadSegment]) -> HashSet<String> {
    segments
        .iter()
        .filter(|segment| segment.source_boundary == PayloadSourceBoundary::Stdio)
        .map(|segment| {
            format!(
                "{}:{}",
                segment.protocol_hint.as_deref().unwrap_or("unknown"),
                payload_direction(segment.direction)
            )
        })
        .collect()
}

pub(super) fn require(
    segments: &[PayloadSegment],
    expected_stdin: &str,
    expected_stdout: &str,
    expected_stderr: &str,
) -> Result<(), String> {
    let mut failures = Vec::new();
    require_stream(
        segments,
        "stdin",
        PayloadDirection::Inbound,
        expected_stdin.as_bytes(),
        &mut failures,
    );
    require_stream(
        segments,
        "stdout",
        PayloadDirection::Outbound,
        expected_stdout.as_bytes(),
        &mut failures,
    );
    require_stream(
        segments,
        "stderr",
        PayloadDirection::Outbound,
        expected_stderr.as_bytes(),
        &mut failures,
    );
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("; "))
    }
}

fn require_stream(
    segments: &[PayloadSegment],
    stream: &'static str,
    direction: PayloadDirection,
    expected_bytes: &[u8],
    failures: &mut Vec<String>,
) {
    let Some(segment) = segments.iter().find(|segment| {
        segment.source_boundary == PayloadSourceBoundary::Stdio
            && segment.direction == direction
            && segment.protocol_hint.as_deref() == Some(stream)
            && contains_bytes(&segment.bytes, expected_bytes)
    }) else {
        failures.push(format!(
            "missing stdio payload stream={stream} direction={}",
            payload_direction(direction)
        ));
        return;
    };
    if segment.captured_size == 0 {
        failures.push(format!("stdio payload {stream} has zero captured size"));
    }
    if segment.content_state != PayloadContentState::Plaintext {
        failures.push(format!("stdio payload {stream} is not plaintext"));
    }
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

fn payload_direction(direction: PayloadDirection) -> &'static str {
    match direction {
        PayloadDirection::Outbound => "outbound",
        PayloadDirection::Inbound => "inbound",
    }
}
