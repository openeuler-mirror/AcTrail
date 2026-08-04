use std::sync::Arc;

use model_core::event::{DomainEvent, EventPayload, IpcPayload};

use super::*;

impl McpStdioSessionRegistry {
    pub(in crate::live::mcp) fn observe_lifecycle_event(
        &mut self,
        event: &DomainEvent,
    ) -> McpStdioLifecycleObservation {
        let EventPayload::Ipc(payload) = &event.payload else {
            return McpStdioLifecycleObservation::default();
        };
        if payload.channel != STDIO_BUNDLE_CHANNEL {
            return McpStdioLifecycleObservation::default();
        }

        let trace_id = event.envelope.trace_id;
        let process = &event.envelope.process;
        let mut observation = McpStdioLifecycleObservation::default();
        match payload.metadata.get("operation").map(String::as_str) {
            Some("lineage_disabled") => {
                observation.removed_sessions = self.remove_trace_sessions(trace_id);
                self.metrics.lifecycle_contract_gaps =
                    self.metrics.lifecycle_contract_gaps.saturating_add(1);
                observation
                    .diagnostics
                    .push(LiveMcpStdioDiagnostic::lifecycle_contract_gap(
                        trace_id,
                        process,
                        event.envelope.observed_at,
                        "lineage_disabled",
                    ));
            }
            Some("lineage_capacity_exhausted") => {
                self.metrics.capacity_exhausted = self.metrics.capacity_exhausted.saturating_add(1);
                observation
                    .diagnostics
                    .push(LiveMcpStdioDiagnostic::capacity_exhausted(
                        trace_id,
                        process,
                        event.envelope.observed_at,
                        "lineage_capacity_exhausted",
                    ));
            }
            Some("ready" | "replaced") => {
                let Some(bundle) = self.valid_bundle(payload, trace_id) else {
                    if let Some(removed) = self.detach_process(trace_id, process) {
                        observation.removed_sessions.push(removed);
                    }
                    self.metrics.lifecycle_contract_gaps =
                        self.metrics.lifecycle_contract_gaps.saturating_add(1);
                    observation
                        .diagnostics
                        .push(LiveMcpStdioDiagnostic::lifecycle_contract_gap(
                            trace_id,
                            process,
                            event.envelope.observed_at,
                            "invalid_stdio_bundle",
                        ));
                    return observation;
                };
                self.bind_bundle(event, bundle, &mut observation);
            }
            Some("closed") => {
                let Some(bundle) = self.valid_bundle(payload, trace_id) else {
                    if let Some(removed) = self.detach_process(trace_id, process) {
                        observation.removed_sessions.push(removed);
                    }
                    self.metrics.lifecycle_contract_gaps =
                        self.metrics.lifecycle_contract_gaps.saturating_add(1);
                    observation
                        .diagnostics
                        .push(LiveMcpStdioDiagnostic::lifecycle_contract_gap(
                            trace_id,
                            process,
                            event.envelope.observed_at,
                            "invalid_closed_stdio_bundle",
                        ));
                    return observation;
                };
                let process_key = (trace_id, process.clone());
                let bundle_is_active = self.sessions_by_process.get(&process_key)
                    == Some(&bundle.session)
                    && self
                        .entries
                        .get(&bundle.session)
                        .and_then(|entry| entry.aliases.get(process))
                        == Some(&bundle.bundle_id);
                if bundle_is_active {
                    self.pending_closures.insert(process_key, bundle.session);
                }
            }
            operation => {
                if let Some(removed) = self.detach_process(trace_id, process) {
                    observation.removed_sessions.push(removed);
                }
                self.metrics.lifecycle_contract_gaps =
                    self.metrics.lifecycle_contract_gaps.saturating_add(1);
                observation
                    .diagnostics
                    .push(LiveMcpStdioDiagnostic::lifecycle_contract_gap(
                        trace_id,
                        process,
                        event.envelope.observed_at,
                        operation.unwrap_or("missing_operation"),
                    ));
            }
        }
        observation
    }

    fn bind_bundle(
        &mut self,
        event: &DomainEvent,
        bundle: McpStdioBundleIdentity,
        observation: &mut McpStdioLifecycleObservation,
    ) {
        let trace_id = event.envelope.trace_id;
        let process = &event.envelope.process;
        let process_key = (trace_id, process.clone());
        self.pending_closures.remove(&process_key);
        if self.sessions_by_process.get(&process_key) != Some(&bundle.session)
            && let Some(removed) = self.detach_process(trace_id, process)
        {
            observation.removed_sessions.push(removed);
        }

        if let Some(entry) = self.entries.get_mut(&bundle.session) {
            entry.aliases.insert(process.clone(), bundle.bundle_id);
        } else {
            let state = if self.pending_candidate_count() >= self.pending_candidate_max_entries {
                self.metrics.capacity_exhausted = self.metrics.capacity_exhausted.saturating_add(1);
                self.record_rejection("candidate_capacity_exhausted");
                observation
                    .diagnostics
                    .push(LiveMcpStdioDiagnostic::capacity_exhausted(
                        trace_id,
                        process,
                        event.envelope.observed_at,
                        "candidate_capacity_exhausted",
                    ));
                McpStdioSessionState::Rejected
            } else {
                self.metrics.candidates = self.metrics.candidates.saturating_add(1);
                self.candidate_keys.insert(bundle.session.clone());
                McpStdioSessionState::Candidate(McpStdioCandidate::new(self.candidate_max_bytes))
            };
            self.entries.insert(
                bundle.session.clone(),
                McpStdioSessionEntry {
                    aliases: BTreeMap::from([(process.clone(), bundle.bundle_id)]),
                    state,
                },
            );
        }
        self.sessions_by_process
            .insert(process_key, bundle.session.clone());
        observation.bound_session = Some(bundle.session);
    }

    fn valid_bundle(
        &self,
        payload: &IpcPayload,
        trace_id: TraceId,
    ) -> Option<McpStdioBundleIdentity> {
        let bundle_id = payload
            .metadata
            .get("bundle_id")
            .filter(|value| !value.is_empty())?
            .clone();
        let stdin_channel_id = payload
            .metadata
            .get("stdin_channel_id")
            .filter(|value| !value.is_empty())?;
        let stdout_channel_id = payload
            .metadata
            .get("stdout_channel_id")
            .filter(|value| !value.is_empty())?;
        let kinds_valid = ["stdin_kind", "stdout_kind"].into_iter().all(|name| {
            payload
                .metadata
                .get(name)
                .is_some_and(|value| matches!(value.as_str(), "pipe" | "unix_socket"))
        });
        let process_valid = payload
            .metadata
            .get("client_host_pid")
            .and_then(|value| value.parse::<u32>().ok())
            .is_some_and(|value| value != 0)
            && payload
                .metadata
                .get("client_generation")
                .and_then(|value| value.parse::<u64>().ok())
                .is_some_and(|value| value != 0)
            && payload
                .metadata
                .get("exec_ktime_ns")
                .and_then(|value| value.parse::<u64>().ok())
                .is_some_and(|value| value != 0);
        (kinds_valid && process_valid).then_some(McpStdioBundleIdentity {
            session: McpStdioSessionKey {
                trace_id,
                stdin_channel_id: Arc::from(stdin_channel_id.as_str()),
                stdout_channel_id: Arc::from(stdout_channel_id.as_str()),
            },
            bundle_id,
        })
    }
}
