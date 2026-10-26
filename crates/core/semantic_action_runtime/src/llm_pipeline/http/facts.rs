//! Bounded joins of semantic projections onto authoritative HTTP message identities.

use crate::live::{HttpResponseMatch, MatchedHttpRequest};
use crate::llm_pipeline::projection::links::{LlmHttpRequestLink, LlmHttpResponseLink};
use crate::llm_pipeline::projection::{ProjectionBatch, llm_call_from_request_response};
use crate::{Http1MessageEvent, Http1MessageHead};
use model_core::payload::{PayloadDirection, PayloadStreamIdentity, PayloadStreamKey};
use semantic_action::{
    SemanticAction, SemanticActionCompleteness, SemanticActionKind, SemanticActionLink,
    SemanticActionLinkOrigin, SemanticActionLinkRole, SemanticActionStatus, SemanticEvidence,
    SemanticEvidenceKind, attr_keys as attrs,
};
use std::collections::BTreeMap;

pub(in crate::llm_pipeline) struct MessageFact {
    pub(in crate::llm_pipeline) identity: PayloadStreamIdentity,
    pub(in crate::llm_pipeline) last_observed: std::time::SystemTime,
    pub(in crate::llm_pipeline) http: SemanticAction,
    pub(in crate::llm_pipeline) semantic: Option<SemanticAction>,
    /// Immutable reverse index of the exchange owner's response binding.
    pub(in crate::llm_pipeline) request_id: Option<String>,
    pub(in crate::llm_pipeline) response_id: Option<String>,
    pub(in crate::llm_pipeline) semantic_finished: bool,
    pub(in crate::llm_pipeline) ended: bool,
    pub(in crate::llm_pipeline) provider_response_id: Option<String>,
}

pub(in crate::llm_pipeline) struct HttpMessageFacts {
    pub(in crate::llm_pipeline) messages: BTreeMap<String, MessageFact>,
    counts: BTreeMap<model_core::ids::TraceId, usize>,
    limit: usize,
    changed: Vec<String>,
    completed: Vec<String>,
}

impl HttpMessageFacts {
    pub(in crate::llm_pipeline) fn trace_messages(
        &self,
        trace_id: model_core::ids::TraceId,
    ) -> impl Iterator<Item = (&String, &MessageFact)> {
        // Both HTTP producers use a trace-delimited message identity. The trailing
        // colon keeps trace 1 separate from trace 10 without a second index.
        let prefix = format!("trace:{}:", trace_id.get());
        self.messages
            .range(prefix.clone()..)
            .take_while(move |(id, _)| id.starts_with(&prefix))
    }

    pub(in crate::llm_pipeline) fn filter_retention(
        mut output: ProjectionBatch,
        retain_http: bool,
    ) -> ProjectionBatch {
        if !retain_http {
            output
                .actions
                .retain(|action| action.kind != SemanticActionKind::HttpMessage);
            output
                .updated_actions
                .retain(|action| action.kind != SemanticActionKind::HttpMessage);
            output
                .updates
                .retain(|update| update.kind != SemanticActionKind::HttpMessage);
            output.links.retain(|link| {
                !matches!(
                    link.role,
                    SemanticActionLinkRole::HttpRequestHttpResponse
                        | SemanticActionLinkRole::LlmRequestHttpMessage
                        | SemanticActionLinkRole::LlmResponseHttpMessage
                        | SemanticActionLinkRole::LlmCallHttpResponse
                )
            });
            output.http_request_links.clear();
            output.http_response_links.clear();
            for action in output.actions.iter_mut().chain(&mut output.updated_actions) {
                action
                    .attributes
                    .remove(attrs::llm_call::HTTP_RESPONSE_ACTION_ID);
            }
        }
        output
    }
    pub(in crate::llm_pipeline) fn new(limit: usize) -> Self {
        Self {
            messages: BTreeMap::new(),
            counts: BTreeMap::new(),
            limit,
            changed: Vec::new(),
            completed: Vec::new(),
        }
    }

    pub(in crate::llm_pipeline) fn remove(&mut self, id: &str) {
        if let Some(fact) = self.messages.remove(id) {
            if let Some(count) = self.counts.get_mut(&fact.http.trace_id) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    self.counts.remove(&fact.http.trace_id);
                }
            }
        }
    }

    pub(in crate::llm_pipeline) fn head(
        &mut self,
        head: &Http1MessageHead,
        output: &mut ProjectionBatch,
    ) -> SemanticAction {
        let source = &head.source;
        if self.counts.get(&source.trace_id).copied().unwrap_or(0) >= self.limit {
            let eviction_id = self
                .trace_messages(source.trace_id)
                .next()
                .map(|(id, _)| id.clone());
            if let Some(id) = eviction_id {
                if let Some(fact) = self.messages.get(&id) {
                    output.diagnostics.push(crate::llm_pipeline::projection::projector::capacity_diagnostic(&fact.http, model_core::diagnostics::LlmPipelineDiagnosticCode::Http1MessageFactsCapacityEvicted));
                    let observed_at = fact.http.end_time.unwrap_or(fact.http.start_time);
                    self.lifecycle(
                        &Http1MessageEvent::Abort {
                            message_id: id.clone(),
                            reason: crate::Http1AbortReason::CapacityExceeded,
                            observed_at,
                        },
                        output,
                    );
                }
                self.remove(&id);
            }
        }
        let mut attributes = BTreeMap::from([
            ("http.message_id".into(), head.message_id.clone()),
            (
                "direction".into(),
                match source.direction {
                    PayloadDirection::Outbound => "outbound",
                    PayloadDirection::Inbound => "inbound",
                }
                .into(),
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
            (
                "source_boundary".into(),
                format!("{:?}", source.source_boundary),
            ),
            (attrs::network::PROTOCOL_NAME.into(), "http".into()),
            (
                attrs::network::PROTOCOL_VERSION.into(),
                head.protocol.into(),
            ),
            (
                attrs::http::OPERATION.into(),
                if source.direction == PayloadDirection::Outbound {
                    "request"
                } else {
                    "response"
                }
                .into(),
            ),
        ]);
        for (key, value) in [
            ("method", &head.method),
            ("target", &head.path),
            ("status_code", &head.status_code),
            ("reason", &head.reason),
        ] {
            if let Some(value) = value {
                attributes.insert(key.into(), value.clone());
            }
        }
        for line in head.headers_text.split("\r\n").skip(1) {
            if let Some((key, value)) = line.split_once(':') {
                if key.eq_ignore_ascii_case("content-type") {
                    attributes.insert("content_type".into(), value.trim().into());
                }
            }
        }
        if let Some(length) = head.declared_body_len {
            attributes.insert("content_length".into(), length.to_string());
        }
        let action = SemanticAction {
            action_id: head.message_id.clone(),
            trace_id: source.trace_id,
            kind: SemanticActionKind::HttpMessage,
            title: if let Some(method) = &head.method {
                format!("{method} {}", head.path.as_deref().unwrap_or(""))
            } else {
                format!("HTTP {}", head.status_code.as_deref().unwrap_or(""))
            },
            start_time: source.observed_at,
            end_time: None,
            process: source.process,
            status: SemanticActionStatus::InProgress,
            completeness: SemanticActionCompleteness::Partial,
            attributes,
            evidence: vec![SemanticEvidence {
                kind: SemanticEvidenceKind::PayloadSegment,
                id: source.segment_id.get(),
                role: "http.message.payload".into(),
            }],
        };
        *self.counts.entry(source.trace_id).or_default() += 1;
        self.messages.insert(
            head.message_id.clone(),
            MessageFact {
                identity: PayloadStreamIdentity {
                    trace_id: source.trace_id,
                    process: source.process,
                    source_boundary: source.source_boundary,
                    stream_key: PayloadStreamKey::new(source.stream_key.clone()),
                },
                last_observed: source.observed_at,
                http: action.clone(),
                semantic: None,
                request_id: None,
                response_id: None,
                semantic_finished: false,
                ended: false,
                provider_response_id: None,
            },
        );
        action
    }

    pub(in crate::llm_pipeline) fn bind(
        &mut self,
        exchange: &HttpResponseMatch,
        output: &mut ProjectionBatch,
    ) {
        let request_id = &exchange.request.action_id;
        let response_id = &exchange.response.action_id;
        let previous = self.call_binding(request_id);
        self.changed.push(request_id.clone());
        if let Some(response) = self.messages.get_mut(response_id) {
            response.request_id = Some(request_id.clone());
            response.http.attributes.insert(
                attrs::http_response::REQUEST_ACTION_ID.into(),
                request_id.clone(),
            );
        }
        if exchange.status_code >= 200 || exchange.status_code == 101 {
            if let Some(request) = self.messages.get_mut(request_id) {
                request.response_id = Some(response_id.clone());
            }
        }
        output.links.push(SemanticActionLink {
            trace_id: exchange.response.trace_id,
            parent_action_id: request_id.clone(),
            child_action_id: response_id.clone(),
            role: SemanticActionLinkRole::HttpRequestHttpResponse,
            origin: SemanticActionLinkOrigin::Observed,
            valid: true,
            evidence: Vec::new(),
            attributes: BTreeMap::new(),
        });
        self.refresh(request_id, previous, output);
    }

    pub(in crate::llm_pipeline) fn bind_stream(
        &mut self,
        request_id: &str,
        response_id: &str,
        output: &mut ProjectionBatch,
    ) {
        let previous = self.call_binding(request_id);
        let Some(request) = self.messages.get_mut(request_id) else {
            return;
        };
        request.response_id = Some(response_id.to_owned());
        let trace_id = request.http.trace_id;
        let Some(response) = self.messages.get_mut(response_id) else {
            return;
        };
        response.request_id = Some(request_id.to_owned());
        response.http.attributes.insert(
            attrs::http_response::REQUEST_ACTION_ID.into(),
            request_id.to_owned(),
        );
        output.updated_actions.push(response.http.clone());
        output.links.push(SemanticActionLink {
            trace_id,
            parent_action_id: request_id.to_owned(),
            child_action_id: response_id.to_owned(),
            role: SemanticActionLinkRole::HttpRequestHttpResponse,
            origin: SemanticActionLinkOrigin::Observed,
            valid: true,
            evidence: Vec::new(),
            attributes: BTreeMap::new(),
        });
        self.changed.push(request_id.to_owned());
        self.refresh(request_id, previous, output);
    }

    pub(in crate::llm_pipeline) fn semantic(
        &mut self,
        source: &str,
        action: SemanticAction,
        finished: bool,
        output: &mut ProjectionBatch,
    ) {
        let previous = self
            .messages
            .get(source)
            .and_then(|fact| self.call_binding(fact.request_id.as_deref().unwrap_or(source)));
        let Some(fact) = self.messages.get_mut(source) else {
            return;
        };
        fact.semantic = Some(crate::live::action_for_live_state(&action));
        fact.semantic_finished |= finished;
        let request_id = fact.request_id.clone();
        self.changed
            .push(request_id.as_deref().unwrap_or(source).to_owned());
        if action.kind == SemanticActionKind::LlmRequest {
            output.http_request_links.push(LlmHttpRequestLink {
                llm_request: action,
                http_request: Self::request(&fact.http),
            });
            self.refresh(source, previous, output);
        } else {
            output.http_response_links.push(LlmHttpResponseLink {
                llm_response: action,
                http_response: fact.http.clone(),
            });
            if let Some(request_id) = request_id {
                self.refresh(&request_id, previous, output);
            }
        }
    }

    pub(in crate::llm_pipeline) fn lifecycle(
        &mut self,
        event: &Http1MessageEvent,
        output: &mut ProjectionBatch,
    ) {
        let policy_limited = matches!(
            event,
            Http1MessageEvent::Abort {
                reason: crate::Http1AbortReason::CapturePolicyLimited,
                ..
            }
        );
        let (id, end, aborted) = match event {
            Http1MessageEvent::End(end) => (&end.message_id, Some(end.observed_at), false),
            Http1MessageEvent::Abort {
                message_id,
                observed_at,
                ..
            } => (message_id, Some(*observed_at), true),
            _ => return,
        };
        let previous = self
            .messages
            .get(id)
            .and_then(|fact| self.call_binding(fact.request_id.as_deref().unwrap_or(id)));
        let Some(fact) = self.messages.get_mut(id) else {
            return;
        };
        fact.ended = true;
        self.completed.push(id.clone());
        let semantic_finished = fact.semantic_finished;
        fact.semantic_finished = true;
        fact.http.end_time = end.or(fact.http.end_time).or(Some(fact.http.start_time));
        if let Http1MessageEvent::Abort { reason, .. } = event {
            if let Some(semantic) = fact.semantic.as_mut() {
                if !semantic_finished
                    && (policy_limited
                        || (semantic.kind == SemanticActionKind::LlmResponse
                            && semantic.status != SemanticActionStatus::Success))
                {
                    let finalization_reason = match reason {
                        crate::Http1AbortReason::TraceClosed => crate::llm_pipeline::stream::finalizer::StreamFinalizationReason::TraceClosed,
                        crate::Http1AbortReason::CapturePolicyLimited => crate::llm_pipeline::stream::finalizer::StreamFinalizationReason::CapturePolicyLimited,
                        _ => crate::llm_pipeline::stream::finalizer::StreamFinalizationReason::OperationIncomplete,
                    };
                    crate::llm_pipeline::stream::finalizer::ResponseFinalizer::finalize_incomplete(
                        semantic,
                        finalization_reason,
                        fact.http.end_time.unwrap_or(fact.http.start_time),
                    );
                    semantic.attributes.insert(
                        "http.message.incomplete_reason".into(),
                        format!("{reason:?}"),
                    );
                    for action in &mut output.actions {
                        if action.action_id == semantic.action_id {
                            crate::llm_pipeline::stream::finalizer::ResponseFinalizer::finalize_incomplete(
                                action,
                                finalization_reason,
                                fact.http.end_time.unwrap_or(fact.http.start_time),
                            );
                            action.attributes.insert(
                                "http.message.incomplete_reason".into(),
                                format!("{reason:?}"),
                            );
                        }
                    }
                    output
                        .updates
                        .push(crate::live::ActionUpdateFactory::lifecycle(semantic, None));
                    output.updated_actions.push(semantic.clone());
                }
            }
            fact.http.attributes.insert(
                "http.message.incomplete_reason".into(),
                format!("{reason:?}"),
            );
        }
        fact.http.status = if (aborted && !policy_limited)
            || fact
                .http
                .attributes
                .get("status_code")
                .and_then(|value| value.parse::<u16>().ok())
                .is_some_and(|status| status >= 400)
        {
            SemanticActionStatus::Error
        } else if fact
            .http
            .attributes
            .get(attrs::network::PROTOCOL_VERSION)
            .is_some_and(|version| version == "h2")
        {
            SemanticActionStatus::Unknown
        } else {
            SemanticActionStatus::Success
        };
        fact.http.completeness = if policy_limited {
            SemanticActionCompleteness::CaptureLimited
        } else if aborted {
            SemanticActionCompleteness::Partial
        } else {
            SemanticActionCompleteness::Complete
        };
        output
            .updates
            .push(crate::live::ActionUpdateFactory::lifecycle(
                &fact.http, None,
            ));
        output.updated_actions.push(fact.http.clone());
        let request_id = fact.request_id.clone();
        self.changed
            .push(request_id.as_deref().unwrap_or(id).to_owned());
        if let Some(request_id) = request_id {
            self.completed.push(request_id.clone());
            self.refresh(&request_id, previous, output);
        } else {
            self.refresh(id, previous, output);
        }
    }

    pub(in crate::llm_pipeline) fn release_completed(&mut self) {
        for id in std::mem::take(&mut self.completed) {
            let Some(fact) = self.messages.get(&id) else {
                continue;
            };
            if !fact.ended {
                continue;
            }
            let aborted = matches!(
                fact.http.completeness,
                SemanticActionCompleteness::Partial | SemanticActionCompleteness::CaptureLimited
            );
            let informational = fact
                .http
                .attributes
                .get("status_code")
                .and_then(|value| value.parse::<u16>().ok())
                .is_some_and(|status| (100..200).contains(&status) && status != 101);
            let response = fact.response_id.clone().filter(|response_id| {
                self.messages
                    .get(response_id)
                    .is_some_and(|response| response.ended)
            });
            if let Some(response) = response {
                self.remove(&response);
                self.remove(&id);
            } else if aborted || informational {
                self.remove(&id);
            }
        }
    }

    pub(in crate::llm_pipeline) fn finalize_missing_responses(
        &self,
        trace_id: model_core::ids::TraceId,
        matches: impl Fn(&MessageFact) -> bool,
        reason: crate::llm_pipeline::stream::finalizer::StreamFinalizationReason,
        output: &mut ProjectionBatch,
    ) {
        if reason != crate::llm_pipeline::stream::finalizer::StreamFinalizationReason::TraceClosed {
            return;
        }
        for (id, fact) in self.trace_messages(trace_id) {
            if fact.response_id.is_some() || !matches(fact) {
                continue;
            }
            let Some(request) = fact
                .semantic
                .as_ref()
                .filter(|action| action.kind == SemanticActionKind::LlmRequest)
            else {
                continue;
            };
            let mut call = llm_call_from_request_response(request, None);
            crate::llm_pipeline::stream::finalizer::ResponseFinalizer::finalize_response_unobserved(
                &mut call, reason,
            );
            call.attributes.insert("http.message_id".into(), id.clone());
            output
                .updates
                .push(crate::live::ActionUpdateFactory::lifecycle(&call, None));
            output.updated_actions.push(call);
        }
    }

    pub(in crate::llm_pipeline) fn merge(&mut self, action: &SemanticAction) -> ProjectionBatch {
        let mut output = ProjectionBatch::default();
        if let Some(fact) = self.messages.get_mut(&action.action_id) {
            let mut observed = fact.http.clone();
            observed.attributes.extend(action.attributes.clone());
            for evidence in &action.evidence {
                if !observed.evidence.contains(evidence) {
                    observed.evidence.push(evidence.clone());
                }
            }
            // Application events own the durable body. Keep only lightweight
            // exchange state after exporting this observation's full view.
            fact.http = crate::live::action_for_live_state(&observed);
            output
                .updates
                .push(crate::live::ActionUpdateFactory::lifecycle(
                    &fact.http, None,
                ));
            output.updated_actions.push(observed);
        }
        output
    }

    pub(in crate::llm_pipeline) fn take_trajectory_bindings(
        &mut self,
    ) -> Vec<(SemanticAction, Option<String>, std::time::SystemTime, bool)> {
        let mut ids = std::mem::take(&mut self.changed);
        ids.sort_unstable();
        ids.dedup();
        ids.into_iter()
            .filter_map(|id| {
                let request = self.messages.get(&id)?;
                let semantic = request.semantic.as_ref()?;
                let response = self.messages.get(request.response_id.as_ref()?)?;
                Some((
                    semantic.clone(),
                    response.provider_response_id.clone(),
                    response.http.end_time.unwrap_or(response.http.start_time),
                    response.http.status == SemanticActionStatus::Error,
                ))
            })
            .collect()
    }

    pub(in crate::llm_pipeline) fn request(http: &SemanticAction) -> MatchedHttpRequest {
        MatchedHttpRequest {
            action_id: http.action_id.clone(),
            evidence: http.evidence.clone(),
            method: http.attributes.get("method").cloned(),
        }
    }
}
