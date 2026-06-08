//! Payload segment JSON rendering.

use model_core::payload::PayloadSegment;

use crate::json;

pub(super) fn payload_json_row(segment: &PayloadSegment) -> String {
    let mut output = payload_base_json(segment);
    output.push('}');
    output
}

pub(super) fn payload_json_with_bytes(segment: &PayloadSegment) -> String {
    let mut output = payload_base_json(segment);
    output.push(',');
    json::field(
        &mut output,
        "text",
        &json::string(&String::from_utf8_lossy(&segment.bytes)),
    );
    output.push('}');
    output
}

fn payload_base_json(segment: &PayloadSegment) -> String {
    let mut output = String::from("{");
    json::field(&mut output, "id", &json::number(segment.segment_id.get()));
    output.push(',');
    json::field(
        &mut output,
        "display_id",
        &json::string(&segment.segment_id.to_string()),
    );
    output.push(',');
    json::field(&mut output, "pid", &json::number(segment.process.pid));
    output.push(',');
    json::field(&mut output, "observed_at", &json::time(segment.observed_at));
    output.push(',');
    json::field(
        &mut output,
        "direction",
        &json::string(&format!("{:?}", segment.direction)),
    );
    output.push(',');
    json::field(
        &mut output,
        "source",
        &json::string(&format!("{:?}", segment.source_boundary)),
    );
    output.push(',');
    json::field(&mut output, "library", &json::string(&segment.library));
    output.push(',');
    json::field(&mut output, "symbol", &json::string(&segment.symbol));
    output.push(',');
    json::field(
        &mut output,
        "protocol_hint",
        &json::optional_string(segment.protocol_hint.as_deref()),
    );
    output.push(',');
    json::field(
        &mut output,
        "captured_size",
        &json::number(segment.captured_size),
    );
    output.push(',');
    json::field(
        &mut output,
        "original_size",
        &json::number(segment.original_size),
    );
    output.push(',');
    json::field(
        &mut output,
        "operation_id",
        &json::number(segment.operation_id),
    );
    output.push(',');
    json::field(
        &mut output,
        "operation_offset",
        &json::number(segment.operation_offset),
    );
    output.push(',');
    json::field(
        &mut output,
        "operation_original_size",
        &json::number(segment.operation_original_size),
    );
    output.push(',');
    json::field(
        &mut output,
        "operation_captured_size",
        &json::number(segment.operation_captured_size),
    );
    output.push(',');
    json::field(
        &mut output,
        "operation_completion_state",
        &json::string(&format!("{:?}", segment.operation_completion_state)),
    );
    output
}
