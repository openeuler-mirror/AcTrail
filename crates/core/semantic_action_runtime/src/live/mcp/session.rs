mod lifecycle;
mod stream;

use std::collections::{BTreeMap, BTreeSet};
use std::time::SystemTime;

use model_core::ids::TraceId;
use model_core::payload::{PayloadContentState, PayloadSegment, PayloadSourceBoundary};
use model_core::process::ProcessIdentity;

use super::diagnostic::{LiveMcpStdioDiagnostic, McpStdioMetrics};
use super::framing::McpJsonRpcFramer;
use super::model::{McpBufferedStdioMessage, McpStdioSessionKey, McpStdioStream};
use stream::McpCandidateSegmentOutcome;

const STDIO_BUNDLE_CHANNEL: &str = "stdio_bundle";

type McpStdioProcessKey = (TraceId, ProcessIdentity);

#[derive(Debug)]
pub(super) struct McpStdioSessionRegistry {
    entries: BTreeMap<McpStdioSessionKey, McpStdioSessionEntry>,
    sessions_by_process: BTreeMap<McpStdioProcessKey, McpStdioSessionKey>,
    candidate_keys: BTreeSet<McpStdioSessionKey>,
    pending_closures: BTreeMap<McpStdioProcessKey, McpStdioSessionKey>,
    candidate_max_bytes: usize,
    parse_buffer_max_bytes: usize,
    pending_candidate_max_entries: usize,
    metrics: McpStdioMetrics,
}

#[derive(Clone, Debug)]
struct McpStdioSessionEntry {
    aliases: BTreeMap<ProcessIdentity, String>,
    state: McpStdioSessionState,
}

#[derive(Clone, Debug)]
enum McpStdioSessionState {
    Candidate(McpStdioCandidate),
    Confirmed(McpConfirmedStdioSession),
    Rejected,
}

#[derive(Clone, Debug)]
struct McpStdioCandidate {
    stdin: McpJsonRpcFramer,
    stdout: McpJsonRpcFramer,
    buffered_messages: Vec<McpBufferedStdioMessage>,
    observed_bytes: usize,
    client_jsonrpc_observed: bool,
}

#[derive(Clone, Debug)]
struct McpConfirmedStdioSession {
    stdin: McpJsonRpcFramer,
    stdout: McpJsonRpcFramer,
}

#[derive(Clone, Debug)]
struct McpStdioBundleIdentity {
    session: McpStdioSessionKey,
    bundle_id: String,
}

#[derive(Default)]
pub(super) struct McpStdioLifecycleObservation {
    pub(super) bound_session: Option<McpStdioSessionKey>,
    pub(super) removed_sessions: Vec<McpStdioSessionKey>,
    pub(super) diagnostics: Vec<LiveMcpStdioDiagnostic>,
}

#[derive(Default)]
pub(super) struct McpStdioRoute {
    pub(super) session: Option<McpStdioSessionKey>,
    pub(super) messages: Vec<McpBufferedStdioMessage>,
    pub(super) payload_segments: Vec<PayloadSegment>,
    pub(super) diagnostics: Vec<LiveMcpStdioDiagnostic>,
}

#[derive(Default)]
pub(super) struct McpStdioSessionDrain {
    pub(super) sessions: Vec<McpStdioSessionKey>,
    pub(super) diagnostics: Vec<LiveMcpStdioDiagnostic>,
}

impl McpStdioSessionRegistry {
    pub(super) fn new(
        candidate_max_bytes: usize,
        parse_buffer_max_bytes: usize,
        pending_candidate_max_entries: usize,
    ) -> Self {
        Self {
            entries: BTreeMap::new(),
            sessions_by_process: BTreeMap::new(),
            candidate_keys: BTreeSet::new(),
            pending_closures: BTreeMap::new(),
            candidate_max_bytes,
            parse_buffer_max_bytes,
            pending_candidate_max_entries,
            metrics: McpStdioMetrics::default(),
        }
    }

    pub(super) fn route_segment(
        &mut self,
        segment: &PayloadSegment,
        retain_evidence: bool,
    ) -> McpStdioRoute {
        if segment.source_boundary != PayloadSourceBoundary::Stdio
            || segment.content_state != PayloadContentState::Plaintext
        {
            return McpStdioRoute::default();
        }
        let stream = McpStdioStream::from_segment(segment);
        if !matches!(stream, McpStdioStream::Stdin | McpStdioStream::Stdout) {
            return McpStdioRoute::default();
        }
        let process_key = (segment.trace_id, segment.process.clone());
        let Some(key) = self.sessions_by_process.get(&process_key).cloned() else {
            self.metrics.untracked_stdio = self.metrics.untracked_stdio.saturating_add(1);
            return McpStdioRoute::default();
        };
        let mut entry = self
            .entries
            .remove(&key)
            .expect("stdio process index must reference a session");
        if !entry.state.accepts_stream(stream) {
            self.entries.insert(key, entry);
            return McpStdioRoute::default();
        }

        let previous = std::mem::replace(&mut entry.state, McpStdioSessionState::Rejected);
        let (state, mut route) = match previous {
            McpStdioSessionState::Candidate(mut candidate) => {
                match self.observe_candidate(&mut candidate, segment, stream, retain_evidence) {
                    Ok(McpCandidateSegmentOutcome::Pending) => (
                        McpStdioSessionState::Candidate(candidate),
                        McpStdioRoute::default(),
                    ),
                    Ok(McpCandidateSegmentOutcome::Confirmed) => {
                        let stdin_draft = if retain_evidence {
                            None
                        } else {
                            candidate.stdin_payload_draft(segment)
                        };
                        let (confirmed, messages) = candidate.confirm(self.parse_buffer_max_bytes);
                        self.candidate_keys.remove(&key);
                        self.metrics.confirmed = self.metrics.confirmed.saturating_add(1);
                        (
                            McpStdioSessionState::Confirmed(confirmed),
                            McpStdioRoute {
                                messages,
                                payload_segments: stdin_draft.into_iter().collect(),
                                ..McpStdioRoute::default()
                            },
                        )
                    }
                    Ok(McpCandidateSegmentOutcome::StreamDiscarded(reason)) => {
                        self.record_candidate_stream_discard(reason);
                        (
                            McpStdioSessionState::Candidate(candidate),
                            McpStdioRoute {
                                diagnostics: vec![
                                    LiveMcpStdioDiagnostic::candidate_stream_discarded(
                                        segment.trace_id,
                                        &segment.process,
                                        segment.observed_at,
                                        reason,
                                        stream,
                                    ),
                                ],
                                ..McpStdioRoute::default()
                            },
                        )
                    }
                    Err(reason) => {
                        self.candidate_keys.remove(&key);
                        self.record_rejection(reason);
                        (
                            McpStdioSessionState::Rejected,
                            McpStdioRoute {
                                diagnostics: vec![LiveMcpStdioDiagnostic::candidate_rejected(
                                    segment.trace_id,
                                    &segment.process,
                                    segment.observed_at,
                                    reason,
                                    Some(stream),
                                )],
                                ..McpStdioRoute::default()
                            },
                        )
                    }
                }
            }
            McpStdioSessionState::Confirmed(mut confirmed) => {
                let (messages, discarded_reason) =
                    confirmed.observe_segment(segment, stream, retain_evidence);
                let diagnostics = if let Some(reason) = discarded_reason {
                    self.metrics.confirmed_parse_discards =
                        self.metrics.confirmed_parse_discards.saturating_add(1);
                    self.record_discard_reason(reason);
                    vec![LiveMcpStdioDiagnostic::confirmed_stream_discarded(
                        segment.trace_id,
                        &segment.process,
                        segment.observed_at,
                        reason,
                        stream,
                    )]
                } else {
                    Vec::new()
                };
                (
                    McpStdioSessionState::Confirmed(confirmed),
                    McpStdioRoute {
                        messages,
                        diagnostics,
                        ..McpStdioRoute::default()
                    },
                )
            }
            McpStdioSessionState::Rejected => unreachable!("rejected sessions do not route"),
        };
        entry.state = state;
        self.entries.insert(key.clone(), entry);
        route.session = Some(key);
        route
    }

    pub(super) fn should_route(&self, segment: &PayloadSegment) -> bool {
        if segment.source_boundary != PayloadSourceBoundary::Stdio
            || segment.content_state != PayloadContentState::Plaintext
        {
            return false;
        }
        let stream = McpStdioStream::from_segment(segment);
        if !matches!(stream, McpStdioStream::Stdin | McpStdioStream::Stdout) {
            return false;
        }
        self.sessions_by_process
            .get(&(segment.trace_id, segment.process.clone()))
            .and_then(|key| self.entries.get(key))
            .is_some_and(|entry| entry.state.accepts_stream(stream))
    }

    pub(super) fn session_for_process(
        &self,
        trace_id: TraceId,
        process: &ProcessIdentity,
    ) -> Option<McpStdioSessionKey> {
        self.sessions_by_process
            .get(&(trace_id, process.clone()))
            .cloned()
    }

    pub(super) fn close_process(&mut self, trace_id: TraceId, process: ProcessIdentity) {
        let process_key = (trace_id, process);
        if let Some(session) = self.sessions_by_process.get(&process_key) {
            self.pending_closures.insert(process_key, session.clone());
        }
    }

    pub(super) fn take_closed_sessions(&mut self, emitted_at: SystemTime) -> McpStdioSessionDrain {
        let pending = std::mem::take(&mut self.pending_closures);
        let mut drain = McpStdioSessionDrain::default();
        for ((trace_id, process), key) in pending {
            if self.sessions_by_process.get(&(trace_id, process.clone())) != Some(&key) {
                continue;
            }
            self.sessions_by_process
                .remove(&(trace_id, process.clone()));
            let remove_session = {
                let entry = self
                    .entries
                    .get_mut(&key)
                    .expect("stdio process index must reference a session");
                entry.aliases.remove(&process);
                entry.aliases.is_empty()
            };
            if !remove_session {
                continue;
            }
            let entry = self
                .remove_entry(&key)
                .expect("empty stdio session must remain indexed");
            if matches!(entry.state, McpStdioSessionState::Candidate(_)) {
                self.record_rejection("candidate_closed_before_confirmation");
                drain
                    .diagnostics
                    .push(LiveMcpStdioDiagnostic::candidate_rejected(
                        trace_id,
                        &process,
                        emitted_at,
                        "candidate_closed_before_confirmation",
                        None,
                    ));
            }
            drain.sessions.push(key);
        }
        drain
    }

    pub(super) fn forget_trace(&mut self, trace_id: TraceId) {
        self.entries.retain(|key, _| key.trace_id != trace_id);
        self.candidate_keys.retain(|key| key.trace_id != trace_id);
        self.sessions_by_process
            .retain(|(candidate, _), _| *candidate != trace_id);
        self.pending_closures
            .retain(|(candidate, _), _| *candidate != trace_id);
    }

    pub(super) fn take_metrics(&mut self) -> McpStdioMetrics {
        std::mem::take(&mut self.metrics)
    }

    fn observe_candidate(
        &self,
        candidate: &mut McpStdioCandidate,
        segment: &PayloadSegment,
        stream: McpStdioStream,
        retain_evidence: bool,
    ) -> Result<McpCandidateSegmentOutcome, &'static str> {
        if stream.expected_payload_direction() != Some(segment.direction) {
            return Err("stdio_direction_mismatch");
        }
        candidate.observed_bytes = candidate
            .observed_bytes
            .checked_add(segment.bytes.len())
            .ok_or("candidate_size_overflow")?;
        if candidate.observed_bytes > self.candidate_max_bytes {
            return Err("candidate_size_limit");
        }
        candidate.observe_segment(segment, stream, retain_evidence)
    }

    fn detach_process(
        &mut self,
        trace_id: TraceId,
        process: &ProcessIdentity,
    ) -> Option<McpStdioSessionKey> {
        let process_key = (trace_id, process.clone());
        self.pending_closures.remove(&process_key);
        let key = self.sessions_by_process.remove(&process_key)?;
        let remove_session = {
            let entry = self
                .entries
                .get_mut(&key)
                .expect("stdio process index must reference a session");
            entry.aliases.remove(process);
            entry.aliases.is_empty()
        };
        remove_session.then(|| {
            self.remove_entry(&key)
                .expect("empty stdio session must remain indexed");
            key
        })
    }

    fn remove_trace_sessions(&mut self, trace_id: TraceId) -> Vec<McpStdioSessionKey> {
        let keys = self
            .entries
            .keys()
            .filter(|key| key.trace_id == trace_id)
            .cloned()
            .collect::<Vec<_>>();
        for key in &keys {
            self.remove_entry(key);
        }
        self.sessions_by_process
            .retain(|(candidate, _), _| *candidate != trace_id);
        self.pending_closures
            .retain(|(candidate, _), _| *candidate != trace_id);
        keys
    }

    fn pending_candidate_count(&self) -> usize {
        self.candidate_keys.len()
    }

    fn remove_entry(&mut self, key: &McpStdioSessionKey) -> Option<McpStdioSessionEntry> {
        self.candidate_keys.remove(key);
        self.entries.remove(key)
    }

    fn record_rejection(&mut self, reason: &'static str) {
        self.metrics.rejected = self.metrics.rejected.saturating_add(1);
        let total = self.metrics.rejection_reasons.entry(reason).or_default();
        *total = total.saturating_add(1);
    }

    fn record_candidate_stream_discard(&mut self, reason: &'static str) {
        self.metrics.candidate_stream_discards =
            self.metrics.candidate_stream_discards.saturating_add(1);
        self.record_discard_reason(reason);
    }

    fn record_discard_reason(&mut self, reason: &'static str) {
        let total = self.metrics.discard_reasons.entry(reason).or_default();
        *total = total.saturating_add(1);
    }
}

impl McpStdioSessionState {
    fn accepts_stream(&self, stream: McpStdioStream) -> bool {
        match self {
            Self::Candidate(candidate) => candidate.accepts_stream(stream),
            Self::Confirmed(_) => {
                matches!(stream, McpStdioStream::Stdin | McpStdioStream::Stdout)
            }
            Self::Rejected => false,
        }
    }
}
