//! Links for SSE protocol details under LLM responses.

use std::collections::{BTreeMap, BTreeSet};

use model_core::ids::TraceId;
use semantic_action::{
    SemanticAction, SemanticActionKind, SemanticActionLink, SemanticActionLinkConfidence,
    SemanticActionLinkRole, attr_keys as attrs,
};

use super::shared::ActionLinkKey;

const ATTR_LLM_RESPONSE_ACTION_ID: &str = attrs::llm_response::ACTION_ID;
const ATTR_SSE_STREAM_ACTION_ID: &str = attrs::sse::STREAM_ACTION_ID;

#[derive(Default)]
pub(super) struct SseLinkProjector {
    responses: BTreeMap<(TraceId, String), SemanticAction>,
    streams: BTreeMap<(TraceId, String), SemanticAction>,
    pending_streams_by_response: BTreeMap<(TraceId, String), Vec<SemanticAction>>,
    pending_events_by_stream: BTreeMap<(TraceId, String), Vec<SemanticAction>>,
    emitted_links: BTreeSet<ActionLinkKey>,
}

impl SseLinkProjector {
    pub(super) fn observe_action(&mut self, action: &SemanticAction) -> Vec<SemanticActionLink> {
        match action.kind {
            SemanticActionKind::LlmResponse => self.observe_response(action),
            SemanticActionKind::SseStream => self.observe_stream(action),
            SemanticActionKind::SseEvent => self.observe_event(action),
            _ => Vec::new(),
        }
    }

    pub(super) fn forget_trace(&mut self, trace_id: TraceId) {
        self.responses
            .retain(|(candidate, _), _| *candidate != trace_id);
        self.streams
            .retain(|(candidate, _), _| *candidate != trace_id);
        self.pending_streams_by_response
            .retain(|(candidate, _), _| *candidate != trace_id);
        self.pending_events_by_stream
            .retain(|(candidate, _), _| *candidate != trace_id);
        self.emitted_links.retain(|key| key.trace_id != trace_id);
    }

    fn observe_response(&mut self, action: &SemanticAction) -> Vec<SemanticActionLink> {
        let key = (action.trace_id, action.action_id.clone());
        self.responses.insert(key.clone(), action.clone());
        let pending = self.pending_streams_by_response.remove(&key);
        pending
            .unwrap_or_default()
            .iter()
            .filter_map(|stream| self.link_response_stream(action, stream))
            .collect()
    }

    fn observe_stream(&mut self, action: &SemanticAction) -> Vec<SemanticActionLink> {
        self.streams
            .insert((action.trace_id, action.action_id.clone()), action.clone());
        let mut links = Vec::new();
        if let Some(response_id) = action.attributes.get(ATTR_LLM_RESPONSE_ACTION_ID) {
            if let Some(response) = self
                .responses
                .get(&(action.trace_id, response_id.clone()))
                .cloned()
            {
                links.extend(self.link_response_stream(&response, action));
            } else {
                self.pending_streams_by_response
                    .entry((action.trace_id, response_id.clone()))
                    .or_default()
                    .push(action.clone());
            }
        }
        let pending = self
            .pending_events_by_stream
            .remove(&(action.trace_id, action.action_id.clone()));
        links.extend(
            pending
                .unwrap_or_default()
                .iter()
                .filter_map(|event| self.link_stream_event(action, event)),
        );
        links
    }

    fn observe_event(&mut self, action: &SemanticAction) -> Vec<SemanticActionLink> {
        let Some(stream_id) = action.attributes.get(ATTR_SSE_STREAM_ACTION_ID) else {
            return Vec::new();
        };
        if let Some(stream) = self
            .streams
            .get(&(action.trace_id, stream_id.clone()))
            .cloned()
        {
            return self
                .link_stream_event(&stream, action)
                .into_iter()
                .collect();
        }
        self.pending_events_by_stream
            .entry((action.trace_id, stream_id.clone()))
            .or_default()
            .push(action.clone());
        Vec::new()
    }

    fn link_response_stream(
        &mut self,
        response: &SemanticAction,
        stream: &SemanticAction,
    ) -> Option<SemanticActionLink> {
        self.link(
            response,
            stream,
            SemanticActionLinkRole::LlmResponseSseStream,
        )
    }

    fn link_stream_event(
        &mut self,
        stream: &SemanticAction,
        event: &SemanticAction,
    ) -> Option<SemanticActionLink> {
        self.link(stream, event, SemanticActionLinkRole::SseStreamEvent)
    }

    fn link(
        &mut self,
        parent: &SemanticAction,
        child: &SemanticAction,
        role: SemanticActionLinkRole,
    ) -> Option<SemanticActionLink> {
        let key = ActionLinkKey {
            trace_id: parent.trace_id,
            parent_action_id: parent.action_id.clone(),
            child_action_id: child.action_id.clone(),
            role,
        };
        if !self.emitted_links.insert(key) {
            return None;
        }
        Some(SemanticActionLink {
            trace_id: parent.trace_id,
            parent_action_id: parent.action_id.clone(),
            child_action_id: child.action_id.clone(),
            role,
            confidence: SemanticActionLinkConfidence::Observed,
            evidence: child.evidence.clone(),
            attributes: BTreeMap::new(),
        })
    }
}
