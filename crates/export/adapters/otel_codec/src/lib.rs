//! OTLP rendering for semantic action export: JSON (`service`) and protobuf
//! (`otel_pb`), sharing one id/span/link derivation so the two are equivalent.

mod otel_pb;
mod serialize;
mod service;

pub use otel_pb::{
    OTLP_PROTOBUF_CONTENT_TYPE, parse_otlp_protobuf_partial_rejected, render_otlp_protobuf,
    render_otlp_protobuf_line,
};
pub use service::{OtelExportError, render_otlp_json, render_otlp_json_line};
