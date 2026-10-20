//! HTTP/1 observation projection from the shared transport lifecycle.

use super::{ApplicationEventDraft, base64_encode};
use config_core::daemon::{
    ApplicationProtocolConfig, HttpBodyRetention, HttpHeadersRetention, SemanticRetentionConfig,
};
use model_core::event::{ApplicationBody, ApplicationPayload};
use model_core::ids::TraceId;
use model_core::payload::PayloadStreamIdentity;
use semantic_action_runtime::{Http1MessageEvent, Http1MessageHead};
use std::collections::BTreeMap;

#[path = "http1/sse.rs"]
mod sse;

pub(super) struct Http1Observer {
    messages: BTreeMap<String, MessageObservation>,
}

struct MessageObservation {
    head: Http1MessageHead,
    body: Option<Vec<u8>>,
    sse_pending: Option<Vec<u8>>,
    body_retention: HttpBodyRetention,
    limited: bool,
}

impl Http1Observer {
    pub(super) fn new() -> Self {
        Self {
            messages: BTreeMap::new(),
        }
    }

    pub(super) fn observe(
        &mut self,
        events: Vec<Http1MessageEvent>,
        config: &ApplicationProtocolConfig,
        retention: &SemanticRetentionConfig,
        consumed_by_llm: bool,
    ) -> Vec<ApplicationEventDraft> {
        let mut drafts = Vec::new();
        for event in events {
            match event {
                Http1MessageEvent::Head(head) => {
                    let body_retention =
                        retention.http_body_content_for_http_message(consumed_by_llm);
                    let fields = MessageObservation::headers(&head);
                    let is_sse = fields
                        .get("content-type")
                        .is_some_and(|v| v.to_ascii_lowercase().contains("text/event-stream"));
                    let message = MessageObservation {
                        head,
                        body: (retention.http_message_summary_enabled()
                            && !matches!(body_retention, HttpBodyRetention::None))
                        .then(Vec::new),
                        sse_pending: (config.sse_enabled && is_sse).then(Vec::new),
                        body_retention,
                        limited: false,
                    };
                    if retention.http_message_summary_enabled() {
                        drafts.push(
                            ApplicationEventDraft::complete(
                                message.payload(config, retention, None),
                            )
                            .at(message.head.source.observed_at)
                            .for_process(message.head.source.process),
                        );
                    }
                    self.messages
                        .insert(message.head.message_id.clone(), message);
                }
                Http1MessageEvent::Body { message_id, bytes } => {
                    if let Some(message) = self.messages.get_mut(&message_id) {
                        message.consume(&bytes, config, &mut drafts);
                    }
                }
                Http1MessageEvent::End(end) => {
                    if let Some(message) = self.messages.remove(&end.message_id) {
                        if retention.http_message_summary_enabled()
                            && (message.body.is_some() || message.limited)
                        {
                            let body = message.retained_body();
                            let mut payload = message.payload(config, retention, body);
                            payload
                                .metadata
                                .insert("http.message.lifecycle".into(), "end".into());
                            drafts.push(
                                (if message.limited {
                                    ApplicationEventDraft::partial(payload)
                                } else {
                                    ApplicationEventDraft::complete(payload)
                                })
                                .at(end.observed_at)
                                .for_process(message.head.source.process),
                            );
                        }
                    }
                }
                Http1MessageEvent::Abort {
                    message_id,
                    reason,
                    observed_at,
                } => {
                    if let Some(message) = self.messages.remove(&message_id) {
                        if retention.http_message_summary_enabled() {
                            let mut payload = message.payload(config, retention, None);
                            payload
                                .metadata
                                .insert("http.message.lifecycle".into(), "abort".into());
                            payload
                                .metadata
                                .insert("http.message.abort_reason".into(), format!("{reason:?}"));
                            drafts.push(
                                ApplicationEventDraft::partial(payload)
                                    .at(observed_at)
                                    .for_process(message.head.source.process),
                            );
                        }
                    }
                }
            }
        }
        drafts
    }

    pub(super) fn forget_trace(&mut self, trace_id: TraceId) {
        self.messages
            .retain(|_, message| message.head.source.trace_id != trace_id);
    }

    pub(super) fn forget_stream(&mut self, identity: &PayloadStreamIdentity) {
        self.messages.retain(|_, message| {
            let source = &message.head.source;
            source.trace_id != identity.trace_id
                || source.process != identity.process
                || source.stream_key != identity.stream_key.as_str()
        });
    }
}

impl MessageObservation {
    fn headers(head: &Http1MessageHead) -> BTreeMap<String, String> {
        head.headers_text
            .lines()
            .skip(1)
            .filter_map(|line| {
                let (name, value) = line.split_once(':')?;
                Some((name.trim().to_ascii_lowercase(), value.trim().to_string()))
            })
            .collect()
    }

    fn consume(
        &mut self,
        bytes: &[u8],
        config: &ApplicationProtocolConfig,
        drafts: &mut Vec<ApplicationEventDraft>,
    ) {
        let limit = usize::try_from(config.sse_max_buffer_bytes).unwrap_or(usize::MAX);
        if let Some(body) = self.body.as_mut() {
            if bytes.len() > limit.saturating_sub(body.len()) {
                self.body = None;
                self.limited = true;
            } else {
                body.extend_from_slice(bytes);
            }
        }
        let Some(pending) = self.sse_pending.as_mut() else {
            return;
        };
        if bytes.len() > limit.saturating_sub(pending.len()) {
            self.sse_pending = None;
            self.limited = true;
            return;
        }
        pending.extend_from_slice(bytes);
        let complete = pending
            .windows(2)
            .rposition(|v| v == b"\n\n")
            .map(|p| p + 2)
            .into_iter()
            .chain(
                pending
                    .windows(4)
                    .rposition(|v| v == b"\r\n\r\n")
                    .map(|p| p + 4),
            )
            .max();
        if let Some(end) = complete {
            if let Ok(text) = std::str::from_utf8(&pending[..end]) {
                match sse::sse_event_payloads(text, config) {
                    Ok(events) => {
                        drafts.extend(events.into_iter().map(ApplicationEventDraft::complete))
                    }
                    Err(_) => self.limited = true,
                }
            } else {
                self.limited = true;
            }
            pending.drain(..end);
        }
    }

    fn retained_body(&self) -> Option<ApplicationBody> {
        let bytes = self.body.as_ref()?;
        if bytes.is_empty() {
            return None;
        }
        match self.body_retention {
            HttpBodyRetention::None => None,
            HttpBodyRetention::Text => std::str::from_utf8(bytes)
                .ok()
                .map(|v| ApplicationBody::Text(v.to_string())),
            HttpBodyRetention::Json => serde_json::from_slice::<serde_json::Value>(bytes)
                .ok()
                .map(|v| ApplicationBody::Json(v.to_string())),
            HttpBodyRetention::Raw => Some(ApplicationBody::Base64(base64_encode(bytes))),
        }
    }

    fn payload(
        &self,
        config: &ApplicationProtocolConfig,
        retention: &SemanticRetentionConfig,
        body: Option<ApplicationBody>,
    ) -> ApplicationPayload {
        let head = &self.head;
        let source = &head.source;
        let mut metadata = BTreeMap::from([
            ("http.message_id".into(), head.message_id.clone()),
            ("http.message.lifecycle".into(), "head".into()),
            (
                "direction".into(),
                format!("{:?}", source.direction).to_lowercase(),
            ),
            (
                "source_boundary".into(),
                format!("{:?}", source.source_boundary),
            ),
            ("stream_key".into(), source.stream_key.clone()),
            (
                "payload_sequence".into(),
                source.payload_sequence.to_string(),
            ),
            (
                "payload_segment_id".into(),
                source.segment_id.get().to_string(),
            ),
        ]);
        let fields = Self::headers(head);
        match retention.http_headers() {
            HttpHeadersRetention::None => {}
            HttpHeadersRetention::Full => {
                metadata.insert(
                    "http.headers_json".into(),
                    serde_json::json!(fields).to_string(),
                );
            }
            HttpHeadersRetention::Metadata => {
                for key in [
                    "host",
                    "content-type",
                    "transfer-encoding",
                    "content-length",
                ] {
                    if key != "host" || config.capture_host {
                        if let Some(value) = fields.get(key) {
                            metadata.insert(key.replace('-', "_"), value.clone());
                        }
                    }
                }
            }
        }
        if self.limited {
            metadata.insert("http.body_content_state".into(), "capture_limited".into());
        }
        if matches!(self.body_retention, HttpBodyRetention::Json)
            && self.body.as_ref().is_some_and(|b| !b.is_empty())
        {
            metadata.insert(
                "http.body_json_state".into(),
                if body.is_some() {
                    "valid"
                } else {
                    "invalid_or_unavailable"
                }
                .into(),
            );
        }
        let (operation, summary) = if let Some(status) = &head.status_code {
            metadata.insert("status_code".into(), status.clone());
            if let Some(reason) = &head.reason {
                metadata.insert("reason".into(), reason.clone());
            }
            (
                "response",
                format!(
                    "{}{}",
                    status,
                    head.reason
                        .as_ref()
                        .map(|v| format!(" {v}"))
                        .unwrap_or_default()
                ),
            )
        } else {
            let method = head.method.clone().unwrap_or_default();
            let target = head.path.clone().unwrap_or_default();
            metadata.insert("method".into(), method.clone());
            metadata.insert("target".into(), target.clone());
            ("request", format!("{method} {target}"))
        };
        ApplicationPayload {
            protocol: head.protocol.into(),
            operation: operation.into(),
            summary,
            body,
            metadata,
        }
    }
}
