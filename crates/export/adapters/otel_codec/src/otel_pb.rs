//! OTLP/protobuf rendering for semantic action export.
//!
//! Mirrors the JSON encoder in `service.rs` field-for-field, reusing the same id
//! derivation and span/link-selection logic so the two wire formats are
//! byte-equivalent in meaning. Uses the official `opentelemetry-proto` message
//! types (prost), so field numbers match the OTLP spec with zero hand-maintained
//! risk.

use std::time::{SystemTime, UNIX_EPOCH};

use model_core::trace::TraceRecord;
use opentelemetry_proto::tonic::collector::trace::v1::{
    ExportTraceServiceRequest, ExportTraceServiceResponse,
};
use opentelemetry_proto::tonic::common::v1::{AnyValue, InstrumentationScope, KeyValue, any_value};
use opentelemetry_proto::tonic::resource::v1::Resource;
use opentelemetry_proto::tonic::trace::v1::{
    ResourceSpans, ScopeSpans, Span, Status, span, status,
};
use prost::Message;
use semantic_action::{
    SemanticAction, SemanticActionKind, SemanticActionLink, SemanticActionStatus,
};

use crate::service::{otel_span_id_u64, otel_trace_id_u128, parent_link, support_links};

/// Content type for the produced bytes: `application/x-protobuf`.
pub const OTLP_PROTOBUF_CONTENT_TYPE: &str = "application/x-protobuf";

/// Decode the rejected span count from an OTLP/HTTP protobuf success response.
/// An empty response is a valid full-success response and returns `None`.
pub fn parse_otlp_protobuf_partial_rejected(body: &[u8]) -> Result<Option<u64>, String> {
    let response = ExportTraceServiceResponse::decode(body)
        .map_err(|error| format!("decode OTLP protobuf response: {error}"))?;
    let Some(partial) = response.partial_success else {
        return Ok(None);
    };
    u64::try_from(partial.rejected_spans)
        .map(Some)
        .map_err(|_| "OTLP partial_success.rejected_spans is negative".to_string())
}

/// Encode a trace and its actions/links as a serialized OTLP
/// `ExportTraceServiceRequest` (protobuf wire bytes).
pub fn render_otlp_protobuf(
    trace: &TraceRecord,
    actions: &[SemanticAction],
    links: &[SemanticActionLink],
) -> Vec<u8> {
    build_export_request(trace, actions, links).encode_to_vec()
}

/// Encode a single action as a serialized OTLP `ExportTraceServiceRequest`.
///
/// Concatenating the bytes of several of these yields one valid, merged request
/// (protobuf repeated fields merge on concatenation) — so the transport can
/// batch protobuf records by appending bytes, with no protobuf types of its own.
pub fn render_otlp_protobuf_line(
    trace: &TraceRecord,
    action: &SemanticAction,
    links: &[SemanticActionLink],
) -> Vec<u8> {
    render_otlp_protobuf(trace, std::slice::from_ref(action), links)
}

pub(crate) fn build_export_request(
    trace: &TraceRecord,
    actions: &[SemanticAction],
    links: &[SemanticActionLink],
) -> ExportTraceServiceRequest {
    let trace_id = otel_trace_id_u128(trace).to_be_bytes().to_vec();
    let spans = actions
        .iter()
        .map(|action| build_span(&trace_id, action, links))
        .collect();

    let mut resource_attrs = vec![
        str_kv("service.name", trace.profile_name.as_str()),
        str_kv("actrail.trace.display_name", trace.display_name.as_str()),
        str_kv("actrail.trace.profile_name", trace.profile_name.as_str()),
        int_kv("actrail.trace.id", trace.trace_id.get() as i64),
    ];
    if let Some(container_id) = trace.root_container_id.as_deref() {
        resource_attrs.push(str_kv("container.id", container_id));
    }
    if let Some(pod_uid) = trace.root_pod_uid.as_deref() {
        resource_attrs.push(str_kv("k8s.pod.uid", pod_uid));
    }
    if let Some(host_id) = trace.root_host_id.as_deref() {
        resource_attrs.push(str_kv("host.id", host_id));
    }

    ExportTraceServiceRequest {
        resource_spans: vec![ResourceSpans {
            resource: Some(Resource {
                attributes: resource_attrs,
                ..Default::default()
            }),
            scope_spans: vec![ScopeSpans {
                scope: Some(InstrumentationScope {
                    name: "actrail.semantic_actions".to_string(),
                    version: env!("CARGO_PKG_VERSION").to_string(),
                    ..Default::default()
                }),
                spans,
                ..Default::default()
            }],
            ..Default::default()
        }],
    }
}

fn build_span(trace_id: &[u8], action: &SemanticAction, links: &[SemanticActionLink]) -> Span {
    let mut attributes = vec![
        str_kv("actrail.action.id", &action.action_id),
        str_kv("actrail.action.kind", action.kind.as_str()),
        str_kv("actrail.action.status", action.status.as_str()),
        str_kv("actrail.action.completeness", action.completeness.as_str()),
        int_kv("actrail.process.id", action.process.get() as i64),
    ];
    for (key, value) in &action.attributes {
        attributes.push(str_kv(key, value));
    }

    let start = unix_nanos(action.start_time);
    let events = action
        .evidence
        .iter()
        .map(|evidence| span::Event {
            time_unix_nano: start,
            name: "actrail.evidence".to_string(),
            attributes: vec![
                str_kv("actrail.evidence.kind", evidence.kind.as_str()),
                int_kv("actrail.evidence.id", evidence.id as i64),
                str_kv("actrail.evidence.role", &evidence.role),
            ],
            ..Default::default()
        })
        .collect();

    let parent = parent_link(action, links);
    let parent_span_id = parent
        .map(|link| {
            otel_span_id_u64(&link.parent_action_id)
                .to_be_bytes()
                .to_vec()
        })
        .unwrap_or_default();
    let span_links = support_links(action, links, parent)
        .map(|link| build_link(trace_id, link))
        .collect();

    Span {
        trace_id: trace_id.to_vec(),
        span_id: otel_span_id_u64(&action.action_id).to_be_bytes().to_vec(),
        parent_span_id,
        name: action.title.clone(),
        kind: span_kind(action.kind) as i32,
        start_time_unix_nano: start,
        end_time_unix_nano: unix_nanos(action.end_time.unwrap_or(action.start_time)),
        attributes,
        events,
        links: span_links,
        status: Some(Status {
            code: status_code(action.status) as i32,
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn build_link(trace_id: &[u8], link: &SemanticActionLink) -> span::Link {
    span::Link {
        trace_id: trace_id.to_vec(),
        span_id: otel_span_id_u64(&link.parent_action_id)
            .to_be_bytes()
            .to_vec(),
        attributes: vec![
            str_kv("actrail.link.role", link.role.as_str()),
            str_kv("actrail.link.confidence", link.confidence.as_str()),
        ],
        ..Default::default()
    }
}

fn str_kv(key: &str, value: &str) -> KeyValue {
    KeyValue {
        key: key.to_string(),
        value: Some(AnyValue {
            value: Some(any_value::Value::StringValue(value.to_string())),
        }),
        ..Default::default()
    }
}

fn int_kv(key: &str, value: i64) -> KeyValue {
    KeyValue {
        key: key.to_string(),
        value: Some(AnyValue {
            value: Some(any_value::Value::IntValue(value)),
        }),
        ..Default::default()
    }
}

fn span_kind(kind: SemanticActionKind) -> span::SpanKind {
    match kind {
        SemanticActionKind::HttpMessage
        | SemanticActionKind::LlmCall
        | SemanticActionKind::LlmRequest
        | SemanticActionKind::LlmResponse => span::SpanKind::Client,
        _ => span::SpanKind::Internal,
    }
}

fn status_code(value: SemanticActionStatus) -> status::StatusCode {
    match value {
        SemanticActionStatus::Success => status::StatusCode::Ok,
        SemanticActionStatus::Error => status::StatusCode::Error,
        SemanticActionStatus::InProgress | SemanticActionStatus::Unknown => {
            status::StatusCode::Unset
        }
    }
}

fn unix_nanos(value: SystemTime) -> u64 {
    value
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or_default()
}
