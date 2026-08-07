//! Process execution and agent identity projection.

use std::collections::{BTreeMap, VecDeque};
use std::time::SystemTime;

use model_core::event::{DomainEvent, EventPayload};
use model_core::ids::TraceId;
use model_core::payload::{PayloadDirection, PayloadSegment, PayloadSourceBoundary};
use model_core::process::ProcessIdentity;
use semantic_action::{
    SemanticAction, SemanticActionKind, SemanticActionStatus, SemanticEvidence, attr_keys as attrs,
};

use super::actions::{
    agent_exit_action, agent_identity_action, event_evidence, process_event_attributes,
    process_exec_action, process_exit_action,
};

const E_BPF_COLLECTOR: &str = "ebpf";
const PROCESS_SECCOMP_COLLECTOR: &str = "process-seccomp";
const SECCOMP_OBSERVED: &str = "seccomp_observed";

type ProcessActionKey = (TraceId, ProcessIdentity);

pub(super) struct AgentProjector {
    pending_execs: BTreeMap<ProcessActionKey, VecDeque<PendingExecIntent>>,
    pending_exec_order: BTreeMap<u64, ProcessActionKey>,
    pending_exec_max_entries: usize,
    next_pending_exec_sequence: u64,
    pending_exec_evictions: u64,
    agent_identities: BTreeMap<ProcessActionKey, SemanticAction>,
    process_exits: BTreeMap<ProcessActionKey, DomainEvent>,
    user_input_by_process: BTreeMap<ProcessActionKey, UserInputState>,
}

const USER_INPUT_BUFFER_MAX_BYTES: usize = 16 * 1024;
const PENDING_USER_INPUT_MAX_ENTRIES: usize = 8;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct UserInputState {
    current: Vec<u8>,
    escape_state: EscapeSequenceState,
    pending: VecDeque<PendingUserInput>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum EscapeSequenceState {
    #[default]
    Ground,
    Escape,
    ControlSequence,
    OperatingSystemCommand,
    OperatingSystemCommandEscape,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PendingUserInput {
    observed_at: SystemTime,
    segment_id: u64,
    normalized_text: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PendingExecIntent {
    sequence: u64,
    observed_at: SystemTime,
    title: String,
    attributes: BTreeMap<String, String>,
    evidence: SemanticEvidence,
}

impl AgentProjector {
    pub(super) fn new(_invocation_enabled: bool, pending_exec_max_entries: u32) -> Self {
        let pending_exec_max_entries = usize::try_from(pending_exec_max_entries)
            .expect("process_seccomp.pending_max_entries must fit usize");
        assert!(
            pending_exec_max_entries > 0,
            "process_seccomp.pending_max_entries must be positive"
        );
        Self {
            pending_execs: BTreeMap::new(),
            pending_exec_order: BTreeMap::new(),
            pending_exec_max_entries,
            next_pending_exec_sequence: 0,
            pending_exec_evictions: 0,
            agent_identities: BTreeMap::new(),
            process_exits: BTreeMap::new(),
            user_input_by_process: BTreeMap::new(),
        }
    }

    pub(super) fn observe_payload_segment(&mut self, segment: &PayloadSegment) {
        if segment.source_boundary != PayloadSourceBoundary::Stdio
            || segment.direction != PayloadDirection::Inbound
            || segment.protocol_hint.as_deref() != Some("stdin")
            || segment.bytes.is_empty()
        {
            return;
        }
        let key = action_key(segment.trace_id, &segment.process);
        let state = self.user_input_by_process.entry(key).or_default();
        for byte in &segment.bytes {
            if consume_escape_sequence(&mut state.escape_state, *byte) {
                continue;
            }
            match *byte {
                0x1b => state.escape_state = EscapeSequenceState::Escape,
                b'\r' | b'\n' => {
                    let normalized_text = normalize_user_text(&state.current);
                    state.current.clear();
                    if normalized_text.is_empty() {
                        continue;
                    }
                    while state.pending.len() >= PENDING_USER_INPUT_MAX_ENTRIES {
                        state.pending.pop_front();
                    }
                    state.pending.push_back(PendingUserInput {
                        observed_at: segment.observed_at,
                        segment_id: segment.segment_id.get(),
                        normalized_text,
                    });
                }
                0x08 | 0x7f => {
                    state.current.pop();
                }
                b'\t' => {
                    if state.current.len() < USER_INPUT_BUFFER_MAX_BYTES {
                        state.current.push(b' ');
                    }
                }
                byte if byte >= 0x20 => {
                    if state.current.len() < USER_INPUT_BUFFER_MAX_BYTES {
                        state.current.push(byte);
                    }
                }
                _ => {}
            }
        }
    }

    pub(super) fn annotate_user_input(&mut self, action: &mut SemanticAction) {
        if action.kind != SemanticActionKind::LlmRequest {
            return;
        }
        let Some(preview) = action
            .attributes
            .get(attrs::llm_request::MESSAGE_PREVIEW)
            .map(|value| normalize_text(value.trim_end_matches("...")))
            .filter(|value| !value.is_empty())
        else {
            return;
        };
        let key = action_key(action.trace_id, &action.process);
        let Some(state) = self.user_input_by_process.get_mut(&key) else {
            return;
        };
        let matched_index = state.pending.iter().rposition(|candidate| {
            candidate.observed_at <= action.start_time
                && text_matches(&candidate.normalized_text, &preview)
        });
        let Some(matched_index) = matched_index else {
            return;
        };
        let mut matched = None;
        for index in 0..=matched_index {
            let candidate = state
                .pending
                .pop_front()
                .expect("matched pending user input must still exist");
            if index == matched_index {
                matched = Some(candidate);
            }
        }
        let Some(matched) = matched else {
            return;
        };
        let Ok(observed_at) = matched.observed_at.duration_since(SystemTime::UNIX_EPOCH) else {
            return;
        };
        action.attributes.insert(
            attrs::agent_turn::USER_INPUT_SOURCE.to_string(),
            "stdio".to_string(),
        );
        action.attributes.insert(
            attrs::agent_turn::USER_INPUT_SEGMENT_ID.to_string(),
            matched.segment_id.to_string(),
        );
        action.attributes.insert(
            attrs::agent_turn::USER_INPUT_OBSERVED_AT_UNIX_NANOS.to_string(),
            observed_at.as_nanos().to_string(),
        );
    }

    pub(super) fn observe_process_exec(&mut self, event: &DomainEvent) -> Vec<SemanticAction> {
        if self.is_exec_intent(event) {
            self.remember_exec_intent(event);
            return Vec::new();
        }
        if !self.is_exec_completion(event) {
            return Vec::new();
        }
        let mut action = process_exec_action(event);
        let key = action_key(action.trace_id, &action.process);
        if let Some(intent) = self.take_matching_exec_intent(&key, &action) {
            intent.apply_to(&mut action);
        }
        vec![action]
    }

    pub(super) fn observe_process_exit(&mut self, event: &DomainEvent) -> Vec<SemanticAction> {
        let EventPayload::Process(payload) = &event.payload else {
            return Vec::new();
        };
        if payload.operation != "exit" {
            return Vec::new();
        }
        let key = action_key(event.envelope.trace_id, &event.envelope.process);
        self.clear_pending_execs(&key);
        if self.process_exits.contains_key(&key) {
            return Vec::new();
        }
        self.process_exits.insert(key.clone(), event.clone());
        let mut actions = vec![process_exit_action(event)];
        if let Some(identity) = self.agent_identities.get(&key) {
            actions.push(agent_exit_action(event, &identity.action_id));
        }
        actions
    }

    pub(super) fn observe_llm_request(&mut self, action: &SemanticAction) -> Vec<SemanticAction> {
        if action.kind != SemanticActionKind::LlmRequest
            || action.status == SemanticActionStatus::Error
        {
            return Vec::new();
        }
        let key = action_key(action.trace_id, &action.process);
        if self.agent_identities.contains_key(&key) {
            return Vec::new();
        }
        let identity = agent_identity_action(action);
        self.agent_identities.insert(key.clone(), identity.clone());
        let mut actions = vec![identity.clone()];
        if let Some(exit) = self.process_exits.get(&key) {
            actions.push(agent_exit_action(exit, &identity.action_id));
        }
        actions
    }

    pub(super) fn forget_trace(&mut self, trace_id: TraceId) {
        let pending_keys = self
            .pending_execs
            .keys()
            .filter(|(candidate, _)| *candidate == trace_id)
            .cloned()
            .collect::<Vec<_>>();
        for key in pending_keys {
            self.clear_pending_execs(&key);
        }
        self.agent_identities
            .retain(|(candidate, _), _| *candidate != trace_id);
        self.process_exits
            .retain(|(candidate, _), _| *candidate != trace_id);
        self.user_input_by_process
            .retain(|(candidate, _), _| *candidate != trace_id);
    }

    pub(super) fn take_pending_exec_evictions(&mut self) -> u64 {
        std::mem::take(&mut self.pending_exec_evictions)
    }

    fn is_exec_intent(&self, event: &DomainEvent) -> bool {
        event.envelope.collector.as_str() == PROCESS_SECCOMP_COLLECTOR
            && matches!(
                &event.payload,
                EventPayload::Process(payload)
                    if payload.operation == "exec"
                        && payload
                            .metadata
                            .get(SECCOMP_OBSERVED)
                            .is_some_and(|value| value == "true")
            )
    }

    fn is_exec_completion(&self, event: &DomainEvent) -> bool {
        event.envelope.collector.as_str() == E_BPF_COLLECTOR
            && matches!(
                &event.payload,
                EventPayload::Process(payload) if payload.operation == "exec"
            )
    }

    fn remember_exec_intent(&mut self, event: &DomainEvent) {
        let EventPayload::Process(payload) = &event.payload else {
            return;
        };
        while self.pending_exec_order.len() >= self.pending_exec_max_entries {
            self.evict_oldest_exec_intent();
        }
        let sequence = self.next_pending_sequence();
        let intent = PendingExecIntent {
            sequence,
            observed_at: event.envelope.observed_at,
            title: payload
                .executable
                .clone()
                .unwrap_or_else(|| format!("exec {}", event.envelope.process)),
            attributes: process_event_attributes(event),
            evidence: event_evidence(event, semantic_action::evidence_roles::process::EXEC_INTENT),
        };
        let key = action_key(event.envelope.trace_id, &event.envelope.process);
        self.pending_execs
            .entry(key.clone())
            .or_default()
            .push_back(intent);
        self.pending_exec_order.insert(sequence, key);
    }

    fn take_matching_exec_intent(
        &mut self,
        key: &ProcessActionKey,
        completion: &SemanticAction,
    ) -> Option<PendingExecIntent> {
        let completion_executable = completion
            .attributes
            .get(semantic_action::attr_keys::process::EXECUTABLE);
        let Some(completion_executable) = completion_executable else {
            let unambiguous = self
                .pending_execs
                .get(key)
                .is_some_and(|pending| pending.len() == 1);
            let intent = self.pop_oldest_exec_intent(key);
            return unambiguous.then_some(intent).flatten();
        };
        let match_info = {
            let pending = self.pending_execs.get(key)?;
            let mut matches = pending.iter().enumerate().filter(|(_, intent)| {
                intent
                    .attributes
                    .get(semantic_action::attr_keys::process::EXECUTABLE)
                    == Some(completion_executable)
            });
            matches
                .next()
                .map(|(matched_index, _)| (matched_index, matches.next().is_none()))
        };
        let Some((matched_index, unambiguous)) = match_info else {
            self.pop_oldest_exec_intent(key);
            return None;
        };
        if matched_index > 0 {
            self.pop_oldest_exec_intent(key);
            return None;
        }

        let mut matched = None;
        let mut empty = false;
        if let Some(pending) = self.pending_execs.get_mut(key) {
            for index in 0..=matched_index {
                let Some(intent) = pending.pop_front() else {
                    break;
                };
                self.pending_exec_order.remove(&intent.sequence);
                if index == matched_index && unambiguous {
                    matched = Some(intent);
                }
            }
            empty = pending.is_empty();
        }
        if empty {
            self.pending_execs.remove(key);
        }
        matched
    }

    fn pop_oldest_exec_intent(&mut self, key: &ProcessActionKey) -> Option<PendingExecIntent> {
        let intent = self
            .pending_execs
            .get_mut(key)
            .and_then(VecDeque::pop_front)?;
        self.pending_exec_order.remove(&intent.sequence);
        if self.pending_execs.get(key).is_some_and(VecDeque::is_empty) {
            self.pending_execs.remove(key);
        }
        Some(intent)
    }

    fn clear_pending_execs(&mut self, key: &ProcessActionKey) {
        let Some(pending) = self.pending_execs.remove(key) else {
            return;
        };
        for intent in pending {
            self.pending_exec_order.remove(&intent.sequence);
        }
    }

    fn evict_oldest_exec_intent(&mut self) {
        let Some((&sequence, key)) = self.pending_exec_order.first_key_value() else {
            return;
        };
        let key = key.clone();
        self.pending_exec_order.remove(&sequence);
        let mut empty = false;
        if let Some(pending) = self.pending_execs.get_mut(&key) {
            if pending
                .front()
                .is_some_and(|intent| intent.sequence == sequence)
            {
                pending.pop_front();
            } else if let Some(index) = pending
                .iter()
                .position(|intent| intent.sequence == sequence)
            {
                pending.remove(index);
            }
            empty = pending.is_empty();
        }
        if empty {
            self.pending_execs.remove(&key);
        }
        self.pending_exec_evictions = self.pending_exec_evictions.saturating_add(1);
    }

    fn next_pending_sequence(&mut self) -> u64 {
        loop {
            let sequence = self.next_pending_exec_sequence;
            self.next_pending_exec_sequence = self.next_pending_exec_sequence.wrapping_add(1);
            if !self.pending_exec_order.contains_key(&sequence) {
                return sequence;
            }
        }
    }
}

fn normalize_user_text(bytes: &[u8]) -> String {
    normalize_text(&String::from_utf8_lossy(bytes))
}

fn normalize_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn text_matches(input: &str, preview: &str) -> bool {
    input == preview || input.starts_with(preview)
}

fn consume_escape_sequence(state: &mut EscapeSequenceState, byte: u8) -> bool {
    use EscapeSequenceState::{
        ControlSequence, Escape, Ground, OperatingSystemCommand, OperatingSystemCommandEscape,
    };
    match *state {
        Ground => false,
        Escape => {
            *state = match byte {
                b'[' => ControlSequence,
                b']' => OperatingSystemCommand,
                _ => Ground,
            };
            true
        }
        ControlSequence => {
            if (0x40..=0x7e).contains(&byte) {
                *state = Ground;
            }
            true
        }
        OperatingSystemCommand => {
            if byte == 0x07 {
                *state = Ground;
            } else if byte == 0x1b {
                *state = OperatingSystemCommandEscape;
            }
            true
        }
        OperatingSystemCommandEscape => {
            *state = if byte == b'\\' {
                Ground
            } else {
                OperatingSystemCommand
            };
            true
        }
    }
}

impl PendingExecIntent {
    fn apply_to(self, action: &mut SemanticAction) {
        action.start_time = self.observed_at.min(action.start_time);
        let completion_has_executable = action
            .attributes
            .contains_key(semantic_action::attr_keys::process::EXECUTABLE);
        if !completion_has_executable {
            action.title = self.title;
        }
        for (key, value) in self.attributes {
            action.attributes.entry(key).or_insert(value);
        }
        action.evidence.insert(0, self.evidence);
    }
}

fn action_key(trace_id: TraceId, process: &ProcessIdentity) -> ProcessActionKey {
    (trace_id, process.clone())
}
