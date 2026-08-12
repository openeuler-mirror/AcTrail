//! Batched live-event persistence for deferred seccomp observations.

use std::collections::{BTreeMap, BTreeSet};

use collector_event::{RawCollectorEvent, RawObservationPayload};
use config_core::daemon::FileMetadataRetention;
use control_contract::reply::ControlError;
use ingest_runtime::{AllowPolicy, IngestPipeline};
use model_core::diagnostics::{DiagnosticKind, DiagnosticRecord};
use model_core::event::{DomainEvent, EventPayload};
use model_core::ids::TraceId;
use model_core::process::{ProcessIdentity, ProcessRecord};
use recording_runtime::{SemanticActionBatch, TraceStateRecord};
use semantic_action::{SemanticActionKind, SemanticActionLinkRole, attr_keys as attrs};
use trace_runtime::registry::TraceRuntime;

use crate::services::attach::StorageAttachService;
use crate::services::identity::RuntimeProcessEventApplier;

impl StorageAttachService {
    pub(in crate::services) fn process_live_event_batch(
        &mut self,
        trace_runtime: &mut TraceRuntime,
        raw_events: Vec<RawCollectorEvent>,
    ) -> Result<(), ControlError> {
        if raw_events.is_empty() {
            return Ok(());
        }
        let input_events = raw_events.len();
        let mut retained_events = 0usize;
        let mut semantic_action_count = 0usize;
        let mut semantic_link_count = 0usize;
        let mut batch = LiveEventBatch::default();
        for mut raw_event in raw_events {
            let observed_at = raw_event.envelope.observed_at;
            let parent_observation = match &raw_event.payload {
                RawObservationPayload::Process { parent, .. } => parent.clone(),
                _ => None,
            };
            let (process, process_record) =
                self.resolve_process_observation(raw_event.envelope.process.clone())?;
            if let Some(record) = process_record {
                batch.process_records.insert(record.identity, record);
            }
            let parent = parent_observation
                .map(|observation| self.resolve_process_observation(observation))
                .transpose()?;
            if let Some((_, Some(record))) = &parent {
                batch
                    .process_records
                    .insert(record.identity, record.clone());
            }
            let parent = parent.map(|(identity, _)| identity);
            let matched =
                RuntimeProcessEventApplier::new(trace_runtime, &mut self.process_registry)
                    .apply(&raw_event, process, parent)?;
            if let Some(matched) = &matched {
                self.attach_mcp_stdio_client_identity(
                    trace_runtime,
                    matched.trace_id,
                    &mut raw_event,
                );
            }
            let matched_trace_id = matched.as_ref().map(|matched| matched.trace_id);
            let event_id = self.next_event_id()?;
            let label_event_id = if self.provider_classification_enabled
                && matches!(&raw_event.payload, RawObservationPayload::Net { .. })
            {
                Some(self.next_event_id()?)
            } else {
                None
            };
            let diagnostic_id = self.next_diagnostic_id()?;
            let pipeline = IngestPipeline::new(AllowPolicy, self.provider_classifier.as_ref());
            let outcome =
                pipeline.process(raw_event, matched, event_id, label_event_id, diagnostic_id);

            if let Some(trace_id) = matched_trace_id {
                self.mark_semantic_projection_dirty(trace_id);
                if outcome
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.kind == DiagnosticKind::RuntimeFatal)
                {
                    trace_runtime
                        .fail_trace(trace_id, observed_at)
                        .map_err(|error| ControlError::new("fail_trace", format!("{error:?}")))?;
                }
                batch.trace_ids.insert(trace_id);
                for event in outcome.events {
                    let observation = self.semantic_actions.observe_event_with_diagnostics(&event);
                    batch.diagnostics.extend(
                        self.materialize_mcp_stdio_diagnostics(observation.mcp_stdio_diagnostics)?,
                    );
                    let output = observation.output;
                    let retain_event = output.retain_event;
                    semantic_action_count =
                        semantic_action_count.saturating_add(output.actions.len());
                    semantic_link_count = semantic_link_count.saturating_add(output.links.len());
                    batch
                        .semantic_actions
                        .extend(SemanticActionBatch::from_action_output(
                            output.actions,
                            output.links,
                            output.file_observation_paths,
                            output.file_path_sets,
                            output.llm_request_contents,
                            output.mcp_jsonrpc_contents,
                            output.payload_segments,
                        ));
                    retained_events = retained_events.saturating_add(output.deferred_events.len());
                    for deferred_event in output.deferred_events {
                        batch
                            .events
                            .push(self.prepare_event_for_storage(deferred_event));
                    }
                    if retain_event {
                        retained_events = retained_events.saturating_add(1);
                        batch.events.push(self.prepare_event_for_storage(event));
                    }
                }
            }
            batch.diagnostics.extend(outcome.diagnostics);
        }

        self.workload_diagnostics.record_event_projection(
            input_events,
            retained_events,
            semantic_action_count,
            semantic_link_count,
        );
        if let Some(&trace_id) = batch.trace_ids.iter().next() {
            Self::propagate_tool_names_to_commands(
                &mut batch.semantic_actions,
                self.pending_tool_names.entry(trace_id).or_default(),
            );
        }
        self.persist_live_event_batch(trace_runtime, batch)
    }

    pub(super) fn persist_observed_event_batch(
        &mut self,
        trace_runtime: &TraceRuntime,
        events: Vec<DomainEvent>,
    ) -> Result<(), ControlError> {
        self.persist_observed_event_batch_with_process_records(trace_runtime, events, Vec::new())
    }

    pub(super) fn persist_observed_event_batch_with_process_records(
        &mut self,
        trace_runtime: &TraceRuntime,
        events: Vec<DomainEvent>,
        process_records: Vec<ProcessRecord>,
    ) -> Result<(), ControlError> {
        if events.is_empty() && process_records.is_empty() {
            return Ok(());
        }
        let input_events = events.len();
        let mut retained_events = 0usize;
        let mut semantic_action_count = 0usize;
        let mut semantic_link_count = 0usize;
        let mut batch = LiveEventBatch {
            process_records: process_records
                .into_iter()
                .map(|record| (record.identity, record))
                .collect(),
            ..LiveEventBatch::default()
        };
        for event in events {
            self.mark_semantic_projection_dirty(event.envelope.trace_id);
            let observation = self.semantic_actions.observe_event_with_diagnostics(&event);
            batch
                .diagnostics
                .extend(self.materialize_mcp_stdio_diagnostics(observation.mcp_stdio_diagnostics)?);
            let output = observation.output;
            let retain_event = output.retain_event;
            semantic_action_count = semantic_action_count.saturating_add(output.actions.len());
            semantic_link_count = semantic_link_count.saturating_add(output.links.len());
            batch
                .semantic_actions
                .extend(SemanticActionBatch::from_action_output(
                    output.actions,
                    output.links,
                    output.file_observation_paths,
                    output.file_path_sets,
                    output.llm_request_contents,
                    output.mcp_jsonrpc_contents,
                    output.payload_segments,
                ));
            retained_events = retained_events.saturating_add(output.deferred_events.len());
            for deferred_event in output.deferred_events {
                batch
                    .events
                    .push(self.prepare_event_for_storage(deferred_event));
            }
            if retain_event {
                retained_events = retained_events.saturating_add(1);
                batch.events.push(self.prepare_event_for_storage(event));
            }
        }
        self.workload_diagnostics.record_event_projection(
            input_events,
            retained_events,
            semantic_action_count,
            semantic_link_count,
        );
        if let Some(&trace_id) = batch.trace_ids.iter().next() {
            Self::propagate_tool_names_to_commands(
                &mut batch.semantic_actions,
                self.pending_tool_names.entry(trace_id).or_default(),
            );
        }
        self.persist_live_event_batch(trace_runtime, batch)
    }

    fn persist_live_event_batch(
        &mut self,
        trace_runtime: &TraceRuntime,
        batch: LiveEventBatch,
    ) -> Result<(), ControlError> {
        let trace_states = self.trace_states_for_persistence(trace_runtime, batch.trace_ids)?;
        self.persist_observed_batch_then_publish(
            trace_runtime,
            batch.events,
            batch.diagnostics,
            batch.semantic_actions,
            trace_states,
            batch.process_records.into_values().collect(),
        )
    }

    fn trace_states_for_persistence(
        &self,
        trace_runtime: &TraceRuntime,
        trace_ids: BTreeSet<TraceId>,
    ) -> Result<Vec<TraceStateRecord>, ControlError> {
        trace_ids
            .into_iter()
            .map(|trace_id| self.trace_state_record_for_persistence(trace_runtime, trace_id))
            .collect()
    }

    fn prepare_event_for_storage(&self, mut event: DomainEvent) -> DomainEvent {
        if !self.file_observation.enabled
            || self.file_observation.metadata_retention == FileMetadataRetention::Full
        {
            return event;
        }
        let EventPayload::File(payload) = &mut event.payload else {
            return event;
        };
        payload.metadata.remove("operation");
        payload.metadata.remove("result");
        remove_if_matches_path(&mut payload.metadata, "raw_path", payload.path.as_deref());
        remove_if_matches_path(&mut payload.metadata, "fd_target", payload.path.as_deref());
        let target_path = payload.metadata.get("target_path").cloned();
        remove_if_matches_path(
            &mut payload.metadata,
            "raw_target_path",
            target_path.as_deref(),
        );
        event
    }

    fn attach_mcp_stdio_client_identity(
        &self,
        trace_runtime: &TraceRuntime,
        trace_id: TraceId,
        raw_event: &mut RawCollectorEvent,
    ) {
        let RawObservationPayload::Ipc {
            channel, metadata, ..
        } = &mut raw_event.payload
        else {
            return;
        };
        if channel != "stdio_bundle"
            || !matches!(
                metadata.get("operation").map(String::as_str),
                Some("ready" | "replaced")
            )
        {
            return;
        }

        metadata.remove(attrs::mcp::CLIENT_PROCESS_ID);
        let Some(client_host_pid) = metadata
            .get("client_host_pid")
            .and_then(|value| value.parse::<u32>().ok())
            .filter(|value| *value != 0)
        else {
            return;
        };
        let Some(client_generation) = metadata
            .get("client_generation")
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| *value != 0)
        else {
            return;
        };
        let Some(client) = self.process_registry.active_host_pid(client_host_pid) else {
            return;
        };
        let Some(host) = self
            .process_registry
            .record(client)
            .and_then(|record| record.host.as_ref())
        else {
            return;
        };
        let expected_generation = host.start_boottime_ns.unwrap_or(host.start_time_ticks);
        if expected_generation == 0
            || expected_generation != client_generation
            || trace_runtime
                .get_trace(trace_id)
                .and_then(|trace| trace.memberships.get(&client))
                .is_none()
        {
            return;
        }
        metadata.insert(
            attrs::mcp::CLIENT_PROCESS_ID.to_string(),
            client.get().to_string(),
        );
    }

    /// 将 LlmResponse 中的工具名传播到 CommandInvocation 上。
    ///
    /// 通过两阶段匹配：
    /// 1. 同 batch 内：通过 semantic_links 中的 LlmCall → LlmResponse → CommandInvocation 链路
    /// 2. 跨 batch：通过 pending_tool_names 缓存（LlmResponse 在后续 batch 到达时填充）
    fn propagate_tool_names_to_commands(
        batch: &mut SemanticActionBatch,
        pending_tool_names: &mut BTreeMap<String, String>,
    ) {
        // 1. 收集 LlmResponse 中的工具名：llm_call_action_id → tool_name
        //    同时缓存到 pending_tool_names 供后续 batch 使用
        let mut response_tool_names: BTreeMap<String, String> = BTreeMap::new();
        for action in batch.actions() {
            if action.kind == SemanticActionKind::LlmResponse {
                if let Some(tool_calls_json) =
                    action.attributes.get(attrs::llm_response::TOOL_CALLS_JSON)
                {
                    if let Ok(tool_calls) =
                        serde_json::from_str::<Vec<serde_json::Value>>(tool_calls_json)
                    {
                        for tc in &tool_calls {
                            if let Some(name) = tc
                                .get("function")
                                .and_then(|f| f.get("name"))
                                .and_then(|n| n.as_str())
                            {
                                response_tool_names
                                    .insert(action.action_id.clone(), name.to_string());
                                break; // 取第一个工具名
                            }
                        }
                    }
                }
            }
        }

        // 2. 通过 links 建立关联链
        //    link: parent_action_id → child_action_id
        let mut llm_call_to_response: BTreeMap<&str, &str> = BTreeMap::new();
        let mut command_to_llm_call: BTreeMap<&str, &str> = BTreeMap::new();
        for link in batch.links() {
            match link.role {
                SemanticActionLinkRole::LlmCallResponse => {
                    // parent = LlmCall, child = LlmResponse
                    llm_call_to_response.insert(&link.parent_action_id, &link.child_action_id);
                }
                SemanticActionLinkRole::CommandContainsLlmCall => {
                    // parent = CommandInvocation, child = LlmCall
                    command_to_llm_call.insert(&link.parent_action_id, &link.child_action_id);
                }
                _ => {}
            }
        }

        // 3. 通过 LlmCall 桥接，找到 CommandInvocation → tool_name 的映射
        let mut command_to_tool_name: BTreeMap<String, String> = BTreeMap::new();
        for (command_id, llm_call_id) in &command_to_llm_call {
            if let Some(response_id) = llm_call_to_response.get(llm_call_id) {
                if let Some(tool_name) = response_tool_names.get(*response_id) {
                    command_to_tool_name.insert(command_id.to_string(), tool_name.clone());
                }
            }
        }

        // 4. 将工具名写入对应 CommandInvocation 的 attributes（同 batch 匹配）
        for action in batch.actions_mut() {
            if action.kind == SemanticActionKind::CommandInvocation {
                if let Some(tool_name) = command_to_tool_name.get(&action.action_id) {
                    action
                        .attributes
                        .insert(attrs::command::TOOL_NAME.to_string(), tool_name.clone());
                }
            }
        }

        // 5. 跨 batch 匹配：用 pending_tool_names 缓存查找尚未匹配的 CommandInvocation
        //    （LlmResponse 在后续 batch 到达时，retroactively 更新之前 batch 中的 CommandInvocation）
        let mut needs_update = false;
        for action in batch.actions() {
            if action.kind == SemanticActionKind::CommandInvocation
                && !action.attributes.contains_key(attrs::command::TOOL_NAME)
            {
                // 该 CommandInvocation 尚未有 tool_name，尝试从缓存中查找
                if let Some(tool_name) = pending_tool_names.get(&action.action_id) {
                    command_to_tool_name.insert(action.action_id.clone(), tool_name.clone());
                    needs_update = true;
                }
            }
        }
        if needs_update {
            for action in batch.actions_mut() {
                if action.kind == SemanticActionKind::CommandInvocation
                    && !action.attributes.contains_key(attrs::command::TOOL_NAME)
                {
                    if let Some(tool_name) = command_to_tool_name.get(&action.action_id) {
                        action
                            .attributes
                            .insert(attrs::command::TOOL_NAME.to_string(), tool_name.clone());
                    }
                }
            }
        }

        // 6. 将当前 batch 中新发现的工具名存入缓存，供后续 batch 使用
        //    （当 LlmResponse 在 batch N 到达，但 CommandInvocation 在 batch N-1 已写入 DB 时，
        //     这里缓存的映射会在 batch N+1 的 CommandInvocation 到达时被使用）
        for (llm_call_id, tool_name) in &response_tool_names {
            // 找到该 LlmResponse 对应的 LlmCall，再找到对应的 CommandInvocation
            if let Some(llm_call_action) = batch.actions().iter().find(|a| {
                a.kind == SemanticActionKind::LlmCall
                    && a.attributes
                        .get(attrs::llm_call::RESPONSE_ACTION_ID)
                        .is_some_and(|id| id == llm_call_id)
            }) {
                // 通过 links 找到 LlmCall 的父 CommandInvocation
                for link in batch.links() {
                    if link.role == SemanticActionLinkRole::CommandContainsLlmCall
                        && link.child_action_id == llm_call_action.action_id
                    {
                        pending_tool_names.insert(link.parent_action_id.clone(), tool_name.clone());
                    }
                }
            }
        }
    }
}

fn remove_if_matches_path(
    metadata: &mut std::collections::BTreeMap<String, String>,
    key: &str,
    canonical: Option<&str>,
) {
    if canonical.is_some_and(|canonical| {
        metadata
            .get(key)
            .is_some_and(|candidate| candidate == canonical)
    }) {
        metadata.remove(key);
    }
}

#[derive(Default)]
struct LiveEventBatch {
    events: Vec<DomainEvent>,
    diagnostics: Vec<DiagnosticRecord>,
    semantic_actions: SemanticActionBatch,
    trace_ids: BTreeSet<TraceId>,
    process_records: BTreeMap<ProcessIdentity, ProcessRecord>,
}
