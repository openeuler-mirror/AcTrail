//! OTLP/protobuf rendering for semantic action export.
//!
//! Mirrors the JSON encoder in `service.rs` field-for-field, reusing the same id
//! derivation and span/link-selection logic so the two wire formats are
//! byte-equivalent in meaning (verified by an equivalence test). Uses the
//! official `opentelemetry-proto` message types (prost), so field numbers match
//! the OTLP spec with zero hand-maintained risk.

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

use crate::service::{
    action_invalidated, otel_span_id_u64, otel_trace_id_u128, parent_link, support_links,
};

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
        .filter(|action| !action_invalidated(action))
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
    if let Some(confidence) = action.confidence_millis {
        attributes.push(int_kv(
            "actrail.action.confidence_millis",
            i64::from(confidence),
        ));
    }
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::time::{Duration, UNIX_EPOCH};

    use model_core::ids::{ProfileName, TraceId, TraceName};
    use model_core::process::ProcessIdentity;
    use model_core::trace::{TraceAlertToken, TraceRecord};
    use prost::Message;
    use semantic_action::{
        SemanticAction, SemanticActionCompleteness, SemanticActionKind, SemanticActionLink,
        SemanticActionLinkConfidence, SemanticActionLinkRole, SemanticActionStatus,
        SemanticEvidence, SemanticEvidenceKind,
    };
    use serde_json::Value;

    use super::{build_export_request, render_otlp_protobuf};
    use crate::service::render_otlp_json;

    // ---- shared fixture: one trace, two linked spans, evidence + attributes ----

    fn fixture() -> (TraceRecord, Vec<SemanticAction>, Vec<SemanticActionLink>) {
        let mut trace = TraceRecord::new(
            TraceId::new(7),
            TraceAlertToken::new([1; 32]),
            ProcessIdentity::new(100),
            TraceName::new("pid-100"),
            ProfileName::new("container-auto"),
            UNIX_EPOCH,
        );
        trace.root_container_id = Some("6bfb54c1b8d9".to_string());

        let parent = SemanticAction {
            action_id: "trace:7:event:1:command.invocation".to_string(),
            trace_id: TraceId::new(7),
            kind: SemanticActionKind::CommandInvocation,
            title: "/usr/bin/curl".to_string(),
            start_time: UNIX_EPOCH + Duration::from_millis(10),
            end_time: Some(UNIX_EPOCH + Duration::from_millis(20)),
            process: ProcessIdentity::new(100),
            status: SemanticActionStatus::Success,
            completeness: SemanticActionCompleteness::Complete,
            confidence_millis: None,
            attributes: BTreeMap::from([("executable".to_string(), "/usr/bin/curl".to_string())]),
            evidence: vec![SemanticEvidence {
                kind: SemanticEvidenceKind::Event,
                id: 1,
                role: "command.invocation".to_string(),
            }],
        };
        let child = SemanticAction {
            action_id: "trace:7:event:2:http.message".to_string(),
            trace_id: TraceId::new(7),
            kind: SemanticActionKind::HttpMessage,
            title: "POST /api/v1/checkout".to_string(),
            start_time: UNIX_EPOCH + Duration::from_millis(15),
            end_time: None,
            process: ProcessIdentity::new(100),
            status: SemanticActionStatus::Error,
            completeness: SemanticActionCompleteness::Partial,
            confidence_millis: Some(900),
            attributes: BTreeMap::from([
                ("method".to_string(), "POST".to_string()),
                (
                    "http.body_text".to_string(),
                    "user=alice&action=purchase".to_string(),
                ),
            ]),
            evidence: vec![SemanticEvidence {
                kind: SemanticEvidenceKind::PayloadSegment,
                id: 42,
                role: "http.message".to_string(),
            }],
        };
        // parent link (selected as parentSpanId) + a second support link.
        let links = vec![
            SemanticActionLink {
                trace_id: TraceId::new(7),
                parent_action_id: parent.action_id.clone(),
                child_action_id: child.action_id.clone(),
                role: SemanticActionLinkRole::CommandContainsCommandInvocation,
                confidence: SemanticActionLinkConfidence::Observed,
                valid: true,
                evidence: Vec::new(),
                attributes: BTreeMap::new(),
            },
            SemanticActionLink {
                trace_id: TraceId::new(7),
                parent_action_id: parent.action_id.clone(),
                child_action_id: child.action_id.clone(),
                role: SemanticActionLinkRole::CommandContainsFileAccess,
                confidence: SemanticActionLinkConfidence::Derived,
                valid: true,
                evidence: Vec::new(),
                attributes: BTreeMap::new(),
            },
        ];
        (trace, vec![parent, child], links)
    }

    // ---- normalization: JSON and protobuf collapse to the same comparable shape ----

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    type CmpAttributes = Vec<(String, String)>;
    type CmpEvent = (String, String, CmpAttributes);
    type CmpLink = (String, String, CmpAttributes);

    #[derive(Debug, PartialEq, Eq)]
    struct CmpSpan {
        trace_id: String,
        span_id: String,
        parent_span_id: String,
        name: String,
        kind: String,
        start: String,
        end: String,
        status: String,
        attrs: CmpAttributes,
        events: Vec<CmpEvent>,
        links: Vec<CmpLink>,
    }

    fn json_attrs(value: &Value) -> Vec<(String, String)> {
        value
            .as_array()
            .unwrap()
            .iter()
            .map(|kv| {
                let key = kv["key"].as_str().unwrap().to_string();
                let v = &kv["value"];
                let s = if let Some(s) = v.get("stringValue") {
                    format!("s:{}", s.as_str().unwrap())
                } else if let Some(i) = v.get("intValue") {
                    format!("i:{}", i.as_str().unwrap())
                } else {
                    format!("?:{v}")
                };
                (key, s)
            })
            .collect()
    }

    fn proto_attrs(kvs: &[super::KeyValue]) -> Vec<(String, String)> {
        use super::any_value::Value as V;
        kvs.iter()
            .map(|kv| {
                let s = match &kv.value.as_ref().unwrap().value {
                    Some(V::StringValue(s)) => format!("s:{s}"),
                    Some(V::IntValue(i)) => format!("i:{i}"),
                    other => format!("?:{other:?}"),
                };
                (kv.key.clone(), s)
            })
            .collect()
    }

    fn json_spans(doc: &Value) -> Vec<CmpSpan> {
        let scope = &doc["resourceSpans"][0]["scopeSpans"][0];
        scope["spans"]
            .as_array()
            .unwrap()
            .iter()
            .map(|sp| CmpSpan {
                trace_id: sp["traceId"].as_str().unwrap().to_string(),
                span_id: sp["spanId"].as_str().unwrap().to_string(),
                parent_span_id: sp["parentSpanId"].as_str().unwrap_or("").to_string(),
                name: sp["name"].as_str().unwrap().to_string(),
                kind: sp["kind"].as_str().unwrap().to_string(),
                start: sp["startTimeUnixNano"].as_str().unwrap().to_string(),
                end: sp["endTimeUnixNano"].as_str().unwrap().to_string(),
                status: sp["status"]["code"].as_str().unwrap().to_string(),
                attrs: json_attrs(&sp["attributes"]),
                events: sp["events"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|ev| {
                        (
                            ev["name"].as_str().unwrap().to_string(),
                            ev["timeUnixNano"].as_str().unwrap().to_string(),
                            json_attrs(&ev["attributes"]),
                        )
                    })
                    .collect(),
                links: sp["links"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|lk| {
                        (
                            lk["traceId"].as_str().unwrap().to_string(),
                            lk["spanId"].as_str().unwrap().to_string(),
                            json_attrs(&lk["attributes"]),
                        )
                    })
                    .collect(),
            })
            .collect()
    }

    fn proto_kind_str(kind: i32) -> String {
        match kind {
            3 => "SPAN_KIND_CLIENT",
            1 => "SPAN_KIND_INTERNAL",
            other => return format!("SPAN_KIND_{other}"),
        }
        .to_string()
    }

    fn proto_status_str(code: i32) -> String {
        match code {
            1 => "STATUS_CODE_OK",
            2 => "STATUS_CODE_ERROR",
            0 => "STATUS_CODE_UNSET",
            other => return format!("STATUS_CODE_{other}"),
        }
        .to_string()
    }

    fn proto_spans(req: &super::ExportTraceServiceRequest) -> Vec<CmpSpan> {
        let scope = &req.resource_spans[0].scope_spans[0];
        scope
            .spans
            .iter()
            .map(|sp| CmpSpan {
                trace_id: hex(&sp.trace_id),
                span_id: hex(&sp.span_id),
                parent_span_id: hex(&sp.parent_span_id),
                name: sp.name.clone(),
                kind: proto_kind_str(sp.kind),
                start: sp.start_time_unix_nano.to_string(),
                end: sp.end_time_unix_nano.to_string(),
                status: proto_status_str(sp.status.as_ref().unwrap().code),
                attrs: proto_attrs(&sp.attributes),
                events: sp
                    .events
                    .iter()
                    .map(|ev| {
                        (
                            ev.name.clone(),
                            ev.time_unix_nano.to_string(),
                            proto_attrs(&ev.attributes),
                        )
                    })
                    .collect(),
                links: sp
                    .links
                    .iter()
                    .map(|lk| {
                        (
                            hex(&lk.trace_id),
                            hex(&lk.span_id),
                            proto_attrs(&lk.attributes),
                        )
                    })
                    .collect(),
            })
            .collect()
    }

    #[test]
    fn protobuf_matches_json_field_for_field() {
        let (trace, actions, links) = fixture();
        let json: Value =
            serde_json::from_str(&render_otlp_json(&trace, &actions, &links).unwrap()).unwrap();
        let req = build_export_request(&trace, &actions, &links);

        // Resource attributes: same key/value set.
        let json_res = json_attrs(&json["resourceSpans"][0]["resource"]["attributes"]);
        let proto_res = proto_attrs(&req.resource_spans[0].resource.as_ref().unwrap().attributes);
        assert_eq!(json_res, proto_res, "resource attributes diverge");

        // Scope.
        let scope = &json["resourceSpans"][0]["scopeSpans"][0]["scope"];
        let proto_scope = req.resource_spans[0].scope_spans[0].scope.as_ref().unwrap();
        assert_eq!(scope["name"].as_str().unwrap(), proto_scope.name);
        assert_eq!(scope["version"].as_str().unwrap(), proto_scope.version);

        // Every span, field for field.
        assert_eq!(
            json_spans(&json),
            proto_spans(&req),
            "protobuf spans diverge from JSON"
        );
    }

    #[test]
    fn protobuf_bytes_round_trip() {
        let (trace, actions, links) = fixture();
        let bytes = render_otlp_protobuf(&trace, &actions, &links);
        assert!(!bytes.is_empty());
        let decoded = super::ExportTraceServiceRequest::decode(bytes.as_slice())
            .expect("wire bytes decode as a valid ExportTraceServiceRequest");
        assert_eq!(
            proto_spans(&decoded),
            proto_spans(&build_export_request(&trace, &actions, &links)),
            "decoded wire bytes differ from the in-memory request"
        );
    }

    #[test]
    fn trace_and_span_ids_are_correct_widths() {
        let (trace, actions, links) = fixture();
        let req = build_export_request(&trace, &actions, &links);
        for span in &req.resource_spans[0].scope_spans[0].spans {
            assert_eq!(span.trace_id.len(), 16, "trace id must be 16 bytes");
            assert_eq!(span.span_id.len(), 8, "span id must be 8 bytes");
        }
    }

    #[test]
    fn span_links_reference_emitted_trace_in_all_encodings() {
        let (trace, actions, links) = fixture();
        let json: Value =
            serde_json::from_str(&render_otlp_json(&trace, &actions, &links).unwrap()).unwrap();
        let protobuf = super::ExportTraceServiceRequest::decode(
            render_otlp_protobuf(&trace, &actions, &links).as_slice(),
        )
        .expect("wire bytes decode as a valid ExportTraceServiceRequest");

        for spans in [json_spans(&json), proto_spans(&protobuf)] {
            let mut link_count = 0;
            for span in spans {
                for (link_trace_id, _, _) in span.links {
                    link_count += 1;
                    assert_eq!(
                        link_trace_id, span.trace_id,
                        "same-trace span link must reference the emitted trace"
                    );
                }
            }
            assert_ne!(link_count, 0, "fixture must emit at least one span link");
        }
    }

    #[test]
    fn invalid_links_are_omitted_from_all_encodings() {
        let (trace, actions, mut links) = fixture();
        for link in &mut links {
            link.valid = false;
        }
        let json: Value =
            serde_json::from_str(&render_otlp_json(&trace, &actions, &links).unwrap()).unwrap();
        let protobuf = build_export_request(&trace, &actions, &links);

        for spans in [json_spans(&json), proto_spans(&protobuf)] {
            for span in spans {
                assert!(span.parent_span_id.is_empty());
                assert!(span.links.is_empty());
            }
        }
    }
}
