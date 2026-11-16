//! Responses protocol lanes bind creates in the server's declared serial order.
use crate::llm_pipeline::projection::compact::CompletedResponse;
use crate::llm_pipeline::projection::projector::{ParsedMessageContext, ParsedProviderStream};
use crate::llm_pipeline::projection::{
    ProjectedResponseBinding, ProjectionBatch, llm_call_from_request_response,
};
use crate::llm_pipeline::stream::finalizer::{ResponseFinalizer, StreamFinalizationReason};
use crate::llm_pipeline::transport::websocket::{WebSocketAbortReason, WebSocketMessageHead};
use config_core::daemon::SemanticRetentionConfig;
use semantic_action::{
    SemanticAction, SemanticActionLink, SemanticActionLinkOrigin, SemanticActionLinkRole,
    SemanticActionStatus,
};
use std::collections::{BTreeMap, VecDeque};

#[derive(Default)]
pub(super) struct Session {
    pub(super) lanes: BTreeMap<String, Lane>,
    pub(super) blocked: bool,
    pub(super) request_count: usize,
    pub(super) response_bindings: Vec<ProjectedResponseBinding>,
}
#[derive(Default)]
pub(super) struct Lane {
    pub(super) pending: VecDeque<Option<SemanticAction>>,
    pub(super) active: Option<Response>,
    pub(super) blocked: bool,
}
pub(super) struct Response {
    pub(super) request: Option<SemanticAction>,
    pub(super) has_create: bool,
    pub(super) provider_id: String,
    pub(super) parser: ParsedProviderStream,
    pub(super) context: ParsedMessageContext,
    pub(super) first: WebSocketMessageHead,
    pub(super) last: WebSocketMessageHead,
    pub(super) message_count: usize,
    pub(super) dirty: bool,
}
impl Session {
    pub(super) fn flush(&mut self, config: &SemanticRetentionConfig, output: &mut ProjectionBatch) {
        for lane in self.lanes.values_mut() {
            if let Some(response) = lane.active.as_mut() {
                if response.dirty {
                    let mut snapshot = response.snapshot(config, None);
                    if let Some(request) = &response.request {
                        self.response_bindings.push(ProjectedResponseBinding {
                            request: request.clone(),
                            completion: config
                                .l0_llm_call
                                .context_compact
                                .enabled
                                .then(|| {
                                    let observation = snapshot.response_observations.pop();
                                    snapshot.actions.first().and_then(|action| {
                                        CompletedResponse::from_terminal_action(action, observation)
                                    })
                                })
                                .flatten(),
                            provider_response_id: (!response.provider_id.is_empty())
                                .then(|| response.provider_id.clone()),
                            observed_at: response.context.last_at,
                            failed: snapshot
                                .actions
                                .first()
                                .is_some_and(|action| action.status == SemanticActionStatus::Error),
                        });
                    }
                    output.extend(snapshot);
                    response.dirty = false;
                }
                if response.parser.terminal() {
                    if response.has_create {
                        self.request_count = self.request_count.saturating_sub(1);
                    }
                    lane.active = None;
                }
            }
        }
        self.lanes
            .retain(|_, lane| lane.blocked || lane.active.is_some() || !lane.pending.is_empty());
    }
    pub(super) fn abort(
        &mut self,
        config: &SemanticRetentionConfig,
        reason: WebSocketAbortReason,
    ) -> ProjectionBatch {
        let mut output = ProjectionBatch::default();
        for lane in self.lanes.values_mut() {
            output.extend(lane.abort(config, reason, &mut self.response_bindings));
        }
        self.lanes.clear();
        self.request_count = 0;
        self.blocked = true;
        output
    }
    pub(super) fn abort_lane(
        &mut self,
        name: &str,
        config: &SemanticRetentionConfig,
        reason: WebSocketAbortReason,
    ) -> ProjectionBatch {
        let Some(lane) = self.lanes.get_mut(name) else {
            return ProjectionBatch::default();
        };
        let count = lane.pending.len()
            + usize::from(
                lane.active
                    .as_ref()
                    .is_some_and(|response| response.has_create),
            );
        let output = lane.abort(config, reason, &mut self.response_bindings);
        self.request_count = self.request_count.saturating_sub(count);
        output
    }
    fn reason(reason: WebSocketAbortReason) -> StreamFinalizationReason {
        match reason {
            WebSocketAbortReason::CaptureGap => StreamFinalizationReason::ConfirmedGap,
            WebSocketAbortReason::CapturePolicyLimited => {
                StreamFinalizationReason::CapturePolicyLimited
            }
            WebSocketAbortReason::CaptureIncomplete => {
                StreamFinalizationReason::OperationIncomplete
            }
            WebSocketAbortReason::CapacityExceeded => StreamFinalizationReason::BufferBytesExceeded,
            WebSocketAbortReason::DecodeFailure => StreamFinalizationReason::ProtocolDecodeFailed,
            WebSocketAbortReason::PeerClosed => StreamFinalizationReason::PeerClosed,
            WebSocketAbortReason::TraceClosed => StreamFinalizationReason::TraceClosed,
        }
    }
    pub(super) fn call(
        request: &SemanticAction,
        response_id: Option<&str>,
        output: &mut ProjectionBatch,
    ) {
        let call = llm_call_from_request_response(request, response_id);
        if let Some(response_id) = response_id {
            Self::link(
                &call,
                response_id,
                SemanticActionLinkRole::LlmCallResponse,
                output,
            );
            Self::link(
                request,
                response_id,
                SemanticActionLinkRole::LlmRequestLlmResponse,
                output,
            );
            output
                .updates
                .push(crate::live::ActionUpdateFactory::lifecycle(&call, None));
            output.updated_actions.push(call);
        } else {
            Self::link(
                &call,
                &request.action_id,
                SemanticActionLinkRole::LlmCallRequest,
                output,
            );
            output.actions.push(call);
        }
    }
    fn link(
        parent: &SemanticAction,
        child_id: &str,
        role: SemanticActionLinkRole,
        output: &mut ProjectionBatch,
    ) {
        output.links.push(SemanticActionLink {
            trace_id: parent.trace_id,
            parent_action_id: parent.action_id.clone(),
            child_action_id: child_id.to_owned(),
            role,
            origin: SemanticActionLinkOrigin::Observed,
            valid: true,
            evidence: Vec::new(),
            attributes: BTreeMap::new(),
        });
    }
}
impl Lane {
    fn abort(
        &mut self,
        config: &SemanticRetentionConfig,
        reason: WebSocketAbortReason,
        bindings: &mut Vec<ProjectedResponseBinding>,
    ) -> ProjectionBatch {
        let mut output = ProjectionBatch::default();
        if let Some(mut response) = self.active.take() {
            let mut snapshot = response.snapshot(config, Some(reason));
            if let Some(request) = response.request {
                bindings.push(ProjectedResponseBinding {
                    request,
                    completion: config
                        .l0_llm_call
                        .context_compact
                        .enabled
                        .then(|| {
                            let observation = snapshot.response_observations.pop();
                            snapshot.actions.first().and_then(|action| {
                                CompletedResponse::from_terminal_action(action, observation)
                            })
                        })
                        .flatten(),
                    provider_response_id: (!response.provider_id.is_empty())
                        .then_some(response.provider_id),
                    observed_at: response.context.last_at,
                    failed: true,
                });
            }
            output.extend(snapshot);
        }
        for request in self.pending.drain(..).flatten() {
            if Session::reason(reason) != StreamFinalizationReason::TraceClosed {
                continue;
            }
            let mut call = llm_call_from_request_response(&request, None);
            ResponseFinalizer::finalize_response_unobserved(&mut call, Session::reason(reason));
            output
                .updates
                .push(crate::live::ActionUpdateFactory::lifecycle(&call, None));
            output.updated_actions.push(call);
        }
        self.blocked = true;
        output
    }
}
impl Response {
    pub(super) fn bind_call(&self, output: &mut ProjectionBatch) {
        if let Some(request) = &self.request {
            Session::call(request, Some(&self.context.action_id), output);
        }
    }

    fn snapshot(
        &mut self,
        config: &SemanticRetentionConfig,
        abort: Option<WebSocketAbortReason>,
    ) -> ProjectionBatch {
        let mut output = self.parser.snapshot(config, &self.context);
        for action in &mut output.actions {
            action.attributes.insert(
                "websocket.connection_id".into(),
                self.first.connection_id.clone(),
            );
            action
                .attributes
                .insert("websocket.message_id".into(), self.first.message_id.clone());
            action.attributes.insert(
                "websocket.first_message_id".into(),
                self.first.message_id.clone(),
            );
            action.attributes.insert(
                "websocket.last_message_id".into(),
                self.last.message_id.clone(),
            );
            action.attributes.insert(
                "websocket.message_count".into(),
                self.message_count.to_string(),
            );
            if let Some(reason) = abort {
                ResponseFinalizer::finalize_incomplete(
                    action,
                    Session::reason(reason),
                    self.context.last_at,
                );
                action
                    .attributes
                    .insert("websocket.sources_complete".into(), "false".into());
            } else {
                action
                    .attributes
                    .insert("websocket.sources_complete".into(), "true".into());
            }
        }
        output
    }
}
