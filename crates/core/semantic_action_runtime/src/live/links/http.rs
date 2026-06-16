use std::collections::{BTreeMap, BTreeSet};

use semantic_action::{
    SemanticAction, SemanticActionKind, SemanticActionLink, SemanticActionLinkConfidence,
    SemanticActionLinkRole, SemanticEvidenceKind,
};

use crate::live::actions::append_missing_evidence;

use super::shared::{ActionLinkKey, SemanticActionKey};

#[derive(Default)]
pub(super) struct HttpMessageLinkProjector {
    llm_actions: BTreeMap<SemanticActionKey, SemanticAction>,
    http_messages: BTreeMap<SemanticActionKey, SemanticAction>,
    emitted_links: BTreeSet<ActionLinkKey>,
}

impl HttpMessageLinkProjector {
    pub(super) fn observe_action(&mut self, action: &SemanticAction) -> Vec<SemanticActionLink> {
        match action.kind {
            SemanticActionKind::LlmRequest | SemanticActionKind::LlmResponse => {
                self.llm_actions
                    .insert(SemanticActionKey::from(action), action.clone());
                let http_messages = self.http_messages.values().cloned().collect::<Vec<_>>();
                http_messages
                    .iter()
                    .filter_map(|http_message| self.link(action, http_message))
                    .collect()
            }
            SemanticActionKind::HttpMessage if http_message_can_link_to_llm(action) => {
                self.http_messages
                    .insert(SemanticActionKey::from(action), action.clone());
                let llm_actions = self.llm_actions.values().cloned().collect::<Vec<_>>();
                llm_actions
                    .iter()
                    .filter_map(|llm_action| self.link(llm_action, action))
                    .collect()
            }
            _ => Vec::new(),
        }
    }

    pub(super) fn forget_trace(&mut self, trace_id: model_core::ids::TraceId) {
        self.llm_actions.retain(|key, _| key.trace_id != trace_id);
        self.http_messages.retain(|key, _| key.trace_id != trace_id);
        self.emitted_links.retain(|key| key.trace_id != trace_id);
    }

    fn link(
        &mut self,
        llm_action: &SemanticAction,
        http_message: &SemanticAction,
    ) -> Option<SemanticActionLink> {
        let role = llm_http_link_role(llm_action, http_message)?;
        if !self.http_message_matches_llm(llm_action, http_message) {
            return None;
        }
        let key = ActionLinkKey {
            trace_id: llm_action.trace_id,
            parent_action_id: llm_action.action_id.clone(),
            child_action_id: http_message.action_id.clone(),
            role,
        };
        if !self.emitted_links.insert(key) {
            return None;
        }
        let mut evidence = http_message.evidence.clone();
        append_missing_evidence(
            &mut evidence,
            &matching_payload_evidence(llm_action, http_message),
        );
        Some(SemanticActionLink {
            trace_id: llm_action.trace_id,
            parent_action_id: llm_action.action_id.clone(),
            child_action_id: http_message.action_id.clone(),
            role,
            confidence: SemanticActionLinkConfidence::Observed,
            evidence,
            attributes: BTreeMap::new(),
        })
    }
    fn http_message_matches_llm(
        &self,
        llm_action: &SemanticAction,
        http_message: &SemanticAction,
    ) -> bool {
        actions_share_payload_segment(llm_action, http_message)
            || self.response_actions_share_stream(llm_action, http_message)
    }

    fn response_actions_share_stream(
        &self,
        llm_action: &SemanticAction,
        http_message: &SemanticAction,
    ) -> bool {
        if !response_stream_candidate(llm_action, http_message) {
            return false;
        }
        if http_payload_sequence(http_message).is_none() {
            return false;
        }
        let Some(response_sequence) = payload_sequence(llm_action) else {
            return false;
        };
        if self.http_request_between(http_message, response_sequence) {
            return false;
        }
        self.http_messages
            .values()
            .filter(|candidate| response_stream_candidate(llm_action, candidate))
            .filter_map(|candidate| Some((http_payload_sequence(candidate)?, candidate)))
            .filter(|(_, candidate)| !self.http_request_between(candidate, response_sequence))
            .max_by(|left, right| {
                response_candidate_rank(response_sequence, left)
                    .cmp(&response_candidate_rank(response_sequence, right))
            })
            .is_some_and(|(_, candidate)| candidate.action_id == http_message.action_id)
    }

    fn http_request_between(&self, http_response: &SemanticAction, response_sequence: u64) -> bool {
        let Some(response_message_sequence) = http_payload_sequence(http_response) else {
            return false;
        };
        let lower = response_message_sequence.min(response_sequence);
        let upper = response_message_sequence.max(response_sequence);
        self.http_messages.values().any(|candidate| {
            candidate.attributes.get("direction").map(String::as_str) == Some("outbound")
                && same_trace_process_stream(http_response, candidate)
                && http_payload_sequence(candidate)
                    .is_some_and(|sequence| lower < sequence && sequence < upper)
        })
    }
}

fn response_candidate_rank<'a>(
    response_sequence: u64,
    candidate: &(u64, &'a SemanticAction),
) -> (std::cmp::Reverse<u64>, &'a str) {
    (
        std::cmp::Reverse(candidate.0.abs_diff(response_sequence)),
        candidate.1.action_id.as_str(),
    )
}

fn http_message_can_link_to_llm(action: &SemanticAction) -> bool {
    action.kind == SemanticActionKind::HttpMessage
        && action.attributes.contains_key("direction")
        && action.attributes.contains_key("payload_segment_id")
}

fn llm_http_link_role(
    llm_action: &SemanticAction,
    http_message: &SemanticAction,
) -> Option<SemanticActionLinkRole> {
    let direction = http_message.attributes.get("direction")?;
    match (llm_action.kind, direction.as_str()) {
        (SemanticActionKind::LlmRequest, "outbound") => {
            Some(SemanticActionLinkRole::LlmRequestHttpMessage)
        }
        (SemanticActionKind::LlmResponse, "inbound") => {
            Some(SemanticActionLinkRole::LlmResponseHttpMessage)
        }
        _ => None,
    }
}

fn actions_share_payload_segment(
    llm_action: &SemanticAction,
    http_message: &SemanticAction,
) -> bool {
    if llm_action.trace_id != http_message.trace_id || llm_action.process != http_message.process {
        return false;
    }
    let Some(payload_segment_id) = http_payload_segment_id(http_message) else {
        return false;
    };
    llm_action.evidence.iter().any(|evidence| {
        evidence.kind == SemanticEvidenceKind::PayloadSegment && evidence.id == payload_segment_id
    }) || payload_aggregate_matches_http_message(llm_action, http_message)
}

fn response_stream_candidate(llm_action: &SemanticAction, http_message: &SemanticAction) -> bool {
    if llm_action.kind != SemanticActionKind::LlmResponse {
        return false;
    }
    if llm_action.trace_id != http_message.trace_id || llm_action.process != http_message.process {
        return false;
    }
    if http_message.attributes.get("direction").map(String::as_str) != Some("inbound") {
        return false;
    }
    if llm_action
        .attributes
        .get("payload.stream_key")
        .zip(http_message.attributes.get("stream_key"))
        .is_none_or(|(left, right)| left != right)
    {
        return false;
    }
    if !http_stream_ids_match(llm_action, http_message) {
        return false;
    }
    response_status_codes_are_compatible(llm_action, http_message)
}

fn http_stream_ids_match(llm_action: &SemanticAction, http_message: &SemanticAction) -> bool {
    match (
        llm_action.attributes.get("http.response.stream_id"),
        http_message.attributes.get("stream_id"),
    ) {
        (Some(response_stream_id), Some(message_stream_id)) => {
            response_stream_id == message_stream_id
        }
        (Some(_), None) => false,
        (None, _) => true,
    }
}

fn response_status_codes_are_compatible(
    llm_action: &SemanticAction,
    http_message: &SemanticAction,
) -> bool {
    match (
        llm_action.attributes.get("http.response.status_code"),
        http_message.attributes.get("status_code"),
    ) {
        (Some(left), Some(right)) => left == right,
        (Some(_), None) => false,
        (None, _) => true,
    }
}

fn same_trace_process_stream(left: &SemanticAction, right: &SemanticAction) -> bool {
    left.trace_id == right.trace_id
        && left.process == right.process
        && left
            .attributes
            .get("stream_key")
            .zip(right.attributes.get("stream_key"))
            .is_some_and(|(left, right)| left == right)
}

fn matching_payload_evidence(
    llm_action: &SemanticAction,
    http_message: &SemanticAction,
) -> Vec<semantic_action::SemanticEvidence> {
    let Some(payload_segment_id) = http_payload_segment_id(http_message) else {
        return Vec::new();
    };
    let exact = llm_action
        .evidence
        .iter()
        .filter(|evidence| {
            evidence.kind == SemanticEvidenceKind::PayloadSegment
                && evidence.id == payload_segment_id
        })
        .cloned()
        .collect::<Vec<_>>();
    if !exact.is_empty() {
        return exact;
    }
    if !payload_aggregate_matches_http_message(llm_action, http_message) {
        return Vec::new();
    }
    llm_action
        .evidence
        .iter()
        .filter(|evidence| evidence.kind == SemanticEvidenceKind::PayloadAggregate)
        .cloned()
        .collect()
}

fn http_payload_segment_id(action: &SemanticAction) -> Option<u64> {
    action.attributes.get("payload_segment_id")?.parse().ok()
}

fn http_payload_sequence(action: &SemanticAction) -> Option<u64> {
    action.attributes.get("payload_sequence")?.parse().ok()
}

fn payload_sequence(action: &SemanticAction) -> Option<u64> {
    action.attributes.get("payload.sequence")?.parse().ok()
}

fn payload_aggregate_matches_http_message(
    llm_action: &SemanticAction,
    http_message: &SemanticAction,
) -> bool {
    if !llm_action
        .evidence
        .iter()
        .any(|evidence| evidence.kind == SemanticEvidenceKind::PayloadAggregate)
    {
        return false;
    }
    if llm_action
        .attributes
        .get("payload.stream_key")
        .zip(http_message.attributes.get("stream_key"))
        .is_none_or(|(left, right)| left != right)
    {
        return false;
    }
    let Some(http_sequence) = http_payload_sequence(http_message) else {
        return false;
    };
    payload_sequence_range(llm_action)
        .is_some_and(|(start, end)| start <= http_sequence && http_sequence <= end)
}

fn payload_sequence_range(action: &SemanticAction) -> Option<(u64, u64)> {
    let start = action
        .attributes
        .get("payload.sequence_start")?
        .parse()
        .ok()?;
    let end = action
        .attributes
        .get("payload.sequence_end")?
        .parse()
        .ok()?;
    Some((start, end))
}
