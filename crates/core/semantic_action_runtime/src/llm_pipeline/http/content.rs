//! Content failures leave their authoritative HTTP message lifecycle intact.
use super::facts::HttpMessageFacts;
use crate::llm_pipeline::projection::ProjectionBatch;
use crate::llm_pipeline::stream::finalizer::{ResponseFinalizer, StreamFinalizationReason};
use semantic_action::SemanticActionStatus;

impl HttpMessageFacts {
    pub(in crate::llm_pipeline) fn finalize_content(
        &mut self,
        id: &str,
        reason: StreamFinalizationReason,
        at: std::time::SystemTime,
        output: &mut ProjectionBatch,
    ) {
        let previous = self
            .messages
            .get(id)
            .and_then(|fact| self.call_binding(fact.request_id.as_deref().unwrap_or(id)));
        let Some(fact) = self.messages.get_mut(id) else {
            return;
        };
        if fact.semantic_finished {
            return;
        }
        let Some(semantic) = fact.semantic.as_mut() else {
            return;
        };
        if semantic.status == SemanticActionStatus::Success {
            return;
        }
        ResponseFinalizer::finalize_incomplete(semantic, reason, at);
        semantic.attributes.insert(
            "llm.content.incomplete_reason".into(),
            format!("{reason:?}"),
        );
        fact.semantic_finished = true;
        for action in &mut output.actions {
            if action.action_id == semantic.action_id {
                action.status = semantic.status;
                action.completeness = semantic.completeness;
                action.end_time = semantic.end_time;
                action.attributes.insert(
                    "llm.content.incomplete_reason".into(),
                    format!("{reason:?}"),
                );
            }
        }
        output
            .updates
            .push(crate::live::ActionUpdateFactory::lifecycle(semantic, None));
        output.updated_actions.push(semantic.clone());
        let request_id = fact.request_id.as_deref().unwrap_or(id).to_owned();
        self.refresh(&request_id, previous, output);
    }
}
