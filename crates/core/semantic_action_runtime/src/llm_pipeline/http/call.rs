//! Derive call changes from HTTP facts during one state transition.

use super::facts::HttpMessageFacts;
use crate::live::ActionUpdateFactory;
use crate::llm_pipeline::projection::links::LlmHttpResponseLink;
use crate::llm_pipeline::projection::projector::{
    HttpResponseFailure, failed_response_for_open_request,
};
use crate::llm_pipeline::projection::{ProjectionBatch, llm_call_from_request_response};
use semantic_action::{SemanticActionKind, attr_keys as attrs};

/// Temporary comparison value, never retained beyond a fact mutation.
#[derive(PartialEq, Eq)]
pub(super) struct CallBinding {
    request_id: String,
    response_id: Option<String>,
    http_response_id: Option<String>,
}

impl HttpMessageFacts {
    pub(super) fn call_binding(&self, request_id: &str) -> Option<CallBinding> {
        let request = self.messages.get(request_id)?;
        let semantic = request
            .semantic
            .as_ref()
            .filter(|action| action.kind == SemanticActionKind::LlmRequest)?;
        let response = request
            .response_id
            .as_ref()
            .and_then(|id| self.messages.get(id));
        Some(CallBinding {
            request_id: semantic.action_id.clone(),
            response_id: response.and_then(|fact| {
                fact.semantic
                    .as_ref()
                    .map(|action| action.action_id.clone())
                    .or_else(|| {
                        fact.ended
                            .then(|| HttpResponseFailure::response_id(&fact.http))
                            .flatten()
                    })
            }),
            http_response_id: response.map(|fact| fact.http.action_id.clone()),
        })
    }

    pub(super) fn refresh(
        &mut self,
        request_id: &str,
        previous: Option<CallBinding>,
        output: &mut ProjectionBatch,
    ) {
        let Some(binding) = self.call_binding(request_id) else {
            return;
        };
        let request = &self.messages[request_id];
        let Some(llm_request) = request.semantic.as_ref() else {
            return;
        };
        let response = request
            .response_id
            .as_ref()
            .and_then(|id| self.messages.get(id));
        // Failure projection and source evidence are independent of call changes.
        let failure = response
            .filter(|response| response.ended && response.semantic.is_none())
            .and_then(|response| {
                failed_response_for_open_request(
                    &response.http,
                    llm_request,
                    &llm_call_from_request_response(llm_request, None),
                )
            });
        if let (Some(mut failed), Some(response)) = (failure, response) {
            failed
                .attributes
                .insert("http.message_id".into(), response.http.action_id.clone());
            output.http_response_links.push(LlmHttpResponseLink {
                llm_response: failed.clone(),
                http_response: response.http.clone(),
            });
            output.actions.push(failed);
        }
        if previous.as_ref() == Some(&binding) {
            return;
        }
        let mut call = llm_call_from_request_response(llm_request, binding.response_id.as_deref());
        call.attributes
            .insert("http.message_id".into(), request_id.into());
        if let Some(response_id) = &binding.http_response_id {
            call.attributes.insert(
                attrs::llm_call::HTTP_RESPONSE_ACTION_ID.into(),
                response_id.clone(),
            );
        }
        if let Some(previous) = previous {
            if previous.response_id.is_some() != binding.response_id.is_some() {
                output
                    .updates
                    .push(ActionUpdateFactory::lifecycle(&call, None));
            }
            // Relationship IDs are persisted by link projection, not snapshots.
            output.updated_actions.push(call);
        } else {
            output.actions.push(call);
        }
    }
}
