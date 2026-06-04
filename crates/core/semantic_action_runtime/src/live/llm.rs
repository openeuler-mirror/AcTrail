//! Live LLM request projection from retained plaintext payload segments.

use std::collections::{BTreeMap, BTreeSet};

use model_core::ids::TraceId;
use model_core::payload::{PayloadDirection, PayloadSegment};
use semantic_action::SemanticAction;

use crate::payload_projection::llm::{
    PayloadStreamGroupKey, project_llm_request_actions_from_segments,
};

#[derive(Default)]
pub(super) struct LiveLlmProjector {
    streams: BTreeMap<PayloadStreamGroupKey, Vec<PayloadSegment>>,
    emitted_actions: BTreeSet<EmittedLlmAction>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct EmittedLlmAction {
    trace_id: TraceId,
    action_id: String,
}

impl LiveLlmProjector {
    pub(super) fn observe_payload_segment(
        &mut self,
        segment: &PayloadSegment,
    ) -> Vec<SemanticAction> {
        if segment.direction != PayloadDirection::Outbound {
            return Vec::new();
        }
        let key = PayloadStreamGroupKey::from_segment(segment);
        let segments = self.streams.entry(key).or_default();
        segments.push(segment.clone());
        project_llm_request_actions_from_segments(segment.trace_id, segments)
            .into_iter()
            .filter(|action| {
                self.emitted_actions.insert(EmittedLlmAction {
                    trace_id: action.trace_id,
                    action_id: action.action_id.clone(),
                })
            })
            .collect()
    }

    pub(super) fn forget_trace(&mut self, trace_id: TraceId) {
        self.streams.retain(|key, _| key.trace_id != trace_id);
        self.emitted_actions
            .retain(|action| action.trace_id != trace_id);
    }
}
