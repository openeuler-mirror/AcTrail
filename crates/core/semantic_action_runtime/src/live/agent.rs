//! Agent identity projection.

use model_core::event::{DomainEvent, EventPayload};
use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;
use semantic_action::{SemanticAction, SemanticActionKind, SemanticActionStatus, SemanticEvidence};
use std::collections::BTreeMap;

use super::actions::{
    ATTR_AGENT_IDENTITY_EVIDENCE_ACTION_ID, ATTR_AGENT_IDENTITY_SOURCE, ATTR_AGENT_IDENTITY_STATUS,
    append_missing_evidence, event_evidence, process_exit_status,
};

const AGENT_IDENTITY_STATUS_OBSERVED: &str = "observed";
const AGENT_IDENTITY_SOURCE_LLM_REQUEST: &str = "llm.request";

pub(super) struct AgentProjector {
    process_execs: BTreeMap<(TraceId, ProcessIdentity), SemanticAction>,
    llm_agents: BTreeMap<(TraceId, ProcessIdentity), AgentEvidence>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AgentEvidence {
    action_id: String,
    evidence: Vec<SemanticEvidence>,
}

impl AgentProjector {
    pub(super) fn new(_invocation_enabled: bool) -> Self {
        Self {
            process_execs: BTreeMap::new(),
            llm_agents: BTreeMap::new(),
        }
    }

    pub(super) fn observe_process_exec(
        &mut self,
        _event: &DomainEvent,
        action: SemanticAction,
    ) -> Vec<SemanticAction> {
        let mut action = self.merge_process_exec(action);
        let key = action_key(action.trace_id, &action.process);
        if let Some(evidence) = self.llm_agents.get(&key) {
            apply_agent_identity(&mut action, evidence);
        }
        self.process_execs.insert(key, action.clone());
        vec![action]
    }

    pub(super) fn observe_process_exit(&mut self, event: &DomainEvent) -> Vec<SemanticAction> {
        let EventPayload::Process(payload) = &event.payload else {
            return Vec::new();
        };
        let key = action_key(event.envelope.trace_id, &event.envelope.process);
        self.process_execs
            .get(&key)
            .cloned()
            .map(|mut action| {
                action.end_time = Some(event.envelope.observed_at);
                action.status = process_exit_status(payload.metadata.get("exit_code"));
                action.evidence.push(event_evidence(event, "process.exit"));
                self.process_execs.insert(key, action.clone());
                action
            })
            .into_iter()
            .collect()
    }

    pub(super) fn observe_llm_request(&mut self, action: &SemanticAction) -> Vec<SemanticAction> {
        if action.kind != SemanticActionKind::LlmRequest
            || action.status == SemanticActionStatus::Error
        {
            return Vec::new();
        }
        let evidence = AgentEvidence {
            action_id: action.action_id.clone(),
            evidence: action.evidence.clone(),
        };
        let key = action_key(action.trace_id, &action.process);
        self.llm_agents.entry(key.clone()).or_insert(evidence);

        let mut actions = Vec::new();
        if let Some(mut process_action) = self.process_execs.get(&key).cloned() {
            if let Some(evidence) = self.llm_agents.get(&key) {
                apply_agent_identity(&mut process_action, evidence);
                self.process_execs
                    .insert(key.clone(), process_action.clone());
                actions.push(process_action);
            }
        }
        actions
    }

    pub(super) fn forget_trace(&mut self, trace_id: TraceId) {
        self.process_execs
            .retain(|(candidate, _), _| *candidate != trace_id);
        self.llm_agents
            .retain(|(candidate, _), _| *candidate != trace_id);
    }

    fn merge_process_exec(&self, mut action: SemanticAction) -> SemanticAction {
        if let Some(existing) = self
            .process_execs
            .get(&action_key(action.trace_id, &action.process))
        {
            for (key, value) in &existing.attributes {
                action
                    .attributes
                    .entry(key.clone())
                    .or_insert_with(|| value.clone());
            }
            if existing
                .attributes
                .get("seccomp_observed")
                .is_some_and(|value| value == "true")
            {
                action.title = existing.title.clone();
            }
            if existing.start_time < action.start_time {
                action.start_time = existing.start_time;
            }
            if existing.end_time.is_some() {
                action.end_time = existing.end_time;
                action.status = existing.status;
            }
            append_missing_evidence(&mut action.evidence, &existing.evidence);
        }
        action
    }
}

fn apply_agent_identity(action: &mut SemanticAction, evidence: &AgentEvidence) {
    action.attributes.insert(
        ATTR_AGENT_IDENTITY_STATUS.to_string(),
        AGENT_IDENTITY_STATUS_OBSERVED.to_string(),
    );
    action.attributes.insert(
        ATTR_AGENT_IDENTITY_SOURCE.to_string(),
        AGENT_IDENTITY_SOURCE_LLM_REQUEST.to_string(),
    );
    action.attributes.insert(
        ATTR_AGENT_IDENTITY_EVIDENCE_ACTION_ID.to_string(),
        evidence.action_id.clone(),
    );
    append_missing_evidence(&mut action.evidence, &evidence.evidence);
}

fn action_key(trace_id: TraceId, process: &ProcessIdentity) -> (TraceId, ProcessIdentity) {
    (trace_id, process.clone())
}
