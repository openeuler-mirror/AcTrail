//! Live MCP stdio projection from retained plaintext payload segments.
//!
//! `mcp.stdin` and `mcp.stdout` are named from the AI agent/client process
//! perspective. Protocol-level `mcp.request`/`mcp.response` are JSON-RPC roles:
//! a tool-call request is carried by client stdout into MCP server stdin, and a
//! tool-call response is carried by server stdout into client stdin.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::SystemTime;

use config_core::daemon::PayloadMcpConfig;
use model_core::event::{DomainEvent, EventPayload};
use model_core::ids::TraceId;
use model_core::payload::{
    PayloadContentState, PayloadDirection, PayloadSegment, PayloadSourceBoundary,
};
use model_core::process::ProcessIdentity;
use semantic_action::{
    SemanticAction, SemanticActionCompleteness, SemanticActionKind, SemanticActionStatus,
    SemanticEvidence, SemanticEvidenceKind, attr_keys as attrs, evidence_roles,
};
use serde_json::Value;

use crate::payload_projection::http::{
    HttpRequestParts, HttpResponseParts, split_request_batch, split_response_batch,
};

use super::actions::{append_missing_evidence, insert_parent_identity_attributes};

const MCP_TRANSPORT_STDIO: &str = "stdio";
const MCP_TRANSPORT_STREAMABLE_HTTP: &str = "streamable_http";
const MCP_EVIDENCE_MODE_RESPONSE_INFERRED_TOOL: &str = "response_inferred_tool";
const MAX_PENDING_HTTP_REQUEST_CONTEXTS: usize = 64;

#[derive(Default)]
pub(super) struct LiveMcpProjector {
    parse_buffer_max_bytes: usize,
    streams: BTreeMap<McpStreamKey, McpStreamState>,
    http_streams: BTreeMap<McpHttpStreamKey, McpHttpStreamState>,
    servers: BTreeMap<McpServerKey, McpServerState>,
    open_calls: BTreeMap<McpRequestKey, SemanticAction>,
    active_tool_calls: BTreeMap<McpActiveToolCallKey, McpActiveToolCall>,
    pending_http_requests: BTreeMap<McpHttpExchangeKey, VecDeque<McpHttpRequestContext>>,
    action_versions: BTreeMap<(TraceId, String), SemanticAction>,
    llm_tool_proposals: BTreeMap<TraceId, Vec<LlmToolCallProposal>>,
    parents: BTreeMap<McpProcessKey, ProcessIdentity>,
    completed_without_parent: BTreeMap<McpProcessKey, BTreeMap<String, SemanticAction>>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct McpServerKey {
    trace_id: TraceId,
    process: ProcessIdentity,
    endpoint: McpEndpointKey,
}

impl McpServerKey {
    fn stdio_from_segment(segment: &PayloadSegment) -> Self {
        Self {
            trace_id: segment.trace_id,
            process: segment.process.clone(),
            endpoint: McpEndpointKey::Stdio,
        }
    }

    fn streamable_http_from_request(segment: &PayloadSegment, request: &HttpRequestParts) -> Self {
        Self {
            trace_id: segment.trace_id,
            process: segment.process.clone(),
            endpoint: McpEndpointKey::StreamableHttp(McpRemoteEndpointKey::from_request(request)),
        }
    }

    fn is_stdio(&self) -> bool {
        self.endpoint == McpEndpointKey::Stdio
    }

    fn with_http_session_id(&self, session_id: Option<String>) -> Self {
        let McpEndpointKey::StreamableHttp(endpoint) = &self.endpoint else {
            return self.clone();
        };
        Self {
            trace_id: self.trace_id,
            process: self.process.clone(),
            endpoint: McpEndpointKey::StreamableHttp(McpRemoteEndpointKey {
                authority: endpoint.authority.clone(),
                path: endpoint.path.clone(),
                session_id,
            }),
        }
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum McpEndpointKey {
    Stdio,
    StreamableHttp(McpRemoteEndpointKey),
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct McpRemoteEndpointKey {
    authority: String,
    path: String,
    session_id: Option<String>,
}

impl McpRemoteEndpointKey {
    fn from_request(request: &HttpRequestParts) -> Self {
        Self {
            authority: request
                .authority
                .clone()
                .unwrap_or_else(|| "unknown".to_string()),
            path: request
                .path
                .clone()
                .unwrap_or_else(|| remote_http2_path_fallback(request.stream_id)),
            session_id: request
                .headers_text
                .as_deref()
                .and_then(|headers| http_header_value(headers, "MCP-Session-Id"))
                .map(ToString::to_string),
        }
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct McpProcessKey {
    trace_id: TraceId,
    pid: u32,
    generation: u64,
}

impl McpProcessKey {
    fn from_server_key(server_key: &McpServerKey) -> Self {
        Self {
            trace_id: server_key.trace_id,
            pid: server_key.process.pid,
            generation: server_key.process.generation,
        }
    }

    fn from_event(event: &DomainEvent) -> Self {
        Self {
            trace_id: event.envelope.trace_id,
            pid: event.envelope.process.pid,
            generation: event.envelope.process.generation,
        }
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct McpStreamKey {
    server: McpServerKey,
    stream_key: String,
    direction: McpStreamDirection,
}

impl McpStreamKey {
    fn from_segment(segment: &PayloadSegment) -> Self {
        Self {
            server: McpServerKey::stdio_from_segment(segment),
            stream_key: segment.stream_key.as_str().to_string(),
            direction: McpStreamDirection::from(segment.direction),
        }
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct McpHttpStreamKey {
    trace_id: TraceId,
    process: ProcessIdentity,
    stream_key: String,
    direction: McpStreamDirection,
}

impl McpHttpStreamKey {
    fn from_segment(segment: &PayloadSegment) -> Self {
        Self {
            trace_id: segment.trace_id,
            process: segment.process.clone(),
            stream_key: segment.stream_key.as_str().to_string(),
            direction: McpStreamDirection::from(segment.direction),
        }
    }

    fn exchange_key(&self, http_stream_id: Option<u32>) -> McpHttpExchangeKey {
        McpHttpExchangeKey {
            trace_id: self.trace_id,
            process: self.process.clone(),
            stream_key: self.stream_key.clone(),
            http_stream_id,
        }
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct McpHttpExchangeKey {
    trace_id: TraceId,
    process: ProcessIdentity,
    stream_key: String,
    http_stream_id: Option<u32>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum McpStreamDirection {
    Inbound,
    Outbound,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum McpMessageDirection {
    Inbound,
    Outbound,
}

impl McpMessageDirection {
    fn as_str(self) -> &'static str {
        match self {
            Self::Inbound => "inbound",
            Self::Outbound => "outbound",
        }
    }
}

impl From<PayloadDirection> for McpStreamDirection {
    fn from(direction: PayloadDirection) -> Self {
        match direction {
            PayloadDirection::Inbound => Self::Inbound,
            PayloadDirection::Outbound => Self::Outbound,
        }
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct McpRequestKey {
    server: McpServerKey,
    request_id: String,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct McpActiveToolCallKey {
    server: McpServerKey,
    stream_pair_key: String,
}

#[derive(Clone, Debug, Default)]
struct McpActiveToolCall {
    request_key: Option<McpRequestKey>,
    next_exchange_index: u64,
    pending_exchanges: BTreeMap<String, u64>,
}

#[derive(Clone, Debug)]
struct McpHttpRequestContext {
    request_id: Option<String>,
    server_key: McpServerKey,
}

#[derive(Clone, Debug, Default)]
struct McpServerState {
    name: Option<String>,
    tools: BTreeSet<String>,
}

#[derive(Clone, Debug)]
struct McpJsonMessage {
    value: Value,
    observed_at: SystemTime,
    evidence: Vec<SemanticEvidence>,
}

#[derive(Clone, Debug)]
enum McpHttpMessage {
    Request(McpHttpRequestMessage),
    Response(McpHttpResponseMessage),
}

#[derive(Clone, Debug)]
struct McpHttpRequestMessage {
    http: HttpRequestParts,
    messages: Vec<Value>,
    observed_at: SystemTime,
    source_boundary: String,
    evidence: Vec<SemanticEvidence>,
}

#[derive(Clone, Debug)]
struct McpHttpResponseMessage {
    http: HttpResponseParts,
    messages: Vec<Value>,
    observed_at: SystemTime,
    evidence: Vec<SemanticEvidence>,
}

#[derive(Clone, Debug)]
struct McpPendingSseResponse {
    http: HttpResponseParts,
    observed_at: SystemTime,
    stream_evidence: Vec<SemanticEvidence>,
    message_evidence: Vec<SemanticEvidence>,
    body: Vec<u8>,
    mode: McpPendingSseMode,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum McpPendingSseMode {
    RawHttp1,
    ChunkedHttp1,
    Accumulated,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LlmToolCallProposal {
    response_action_id: String,
    tool_call_id: Option<String>,
    name: String,
    server_name: String,
    tool_name: String,
}

impl LiveMcpProjector {
    pub(super) fn new(config: PayloadMcpConfig) -> Self {
        Self {
            parse_buffer_max_bytes: usize::try_from(config.parse_buffer_max_bytes)
                .unwrap_or(usize::MAX),
            ..Self::default()
        }
    }

    pub(super) fn observe_event(&mut self, event: &DomainEvent) -> Vec<SemanticAction> {
        let EventPayload::Process(payload) = &event.payload else {
            return Vec::new();
        };
        if payload.operation != "exec" {
            return Vec::new();
        }
        let Some(parent) = &payload.parent else {
            return Vec::new();
        };
        let process_key = McpProcessKey::from_event(event);
        self.parents.insert(process_key.clone(), parent.clone());
        self.apply_late_parent_identity(&process_key)
    }

    pub(super) fn observe_llm_actions(&mut self, actions: &[SemanticAction]) {
        for action in actions {
            if action.kind != SemanticActionKind::LlmResponse {
                continue;
            }
            let proposals = llm_tool_call_proposals(action);
            if proposals.is_empty() {
                continue;
            }
            let trace_proposals = self.llm_tool_proposals.entry(action.trace_id).or_default();
            trace_proposals.retain(|proposal| proposal.response_action_id != action.action_id);
            trace_proposals.extend(proposals);
        }
    }

    pub(super) fn observe_payload_segment(
        &mut self,
        segment: &PayloadSegment,
    ) -> Vec<SemanticAction> {
        if segment.source_boundary == PayloadSourceBoundary::Stdio
            && segment.content_state == PayloadContentState::Plaintext
        {
            let stream_key = McpStreamKey::from_segment(segment);
            let parse_buffer_max_bytes = self.parse_buffer_max_bytes;
            let messages = self
                .streams
                .entry(stream_key.clone())
                .or_insert_with(|| McpStreamState::new(parse_buffer_max_bytes))
                .observe_segment(segment);
            let mut actions = Vec::new();
            for message in messages {
                actions.extend(self.observe_message(
                    &stream_key.server,
                    stream_key.direction,
                    &stream_key.stream_key,
                    message,
                ));
            }
            return actions;
        }

        if !plaintext_http_candidate(segment) {
            return Vec::new();
        }
        self.observe_http_payload_segment(segment)
    }

    pub(super) fn finalize_trace(
        &mut self,
        trace_id: TraceId,
        finished_at: SystemTime,
    ) -> Vec<SemanticAction> {
        let request_keys = self
            .open_calls
            .keys()
            .filter(|key| key.server.trace_id == trace_id)
            .cloned()
            .collect::<Vec<_>>();
        let mut finalized = Vec::new();
        for key in request_keys {
            let Some(mut action) = self.open_calls.remove(&key) else {
                continue;
            };
            action.status = SemanticActionStatus::Error;
            action.completeness = SemanticActionCompleteness::Partial;
            action.end_time = Some(finished_at);
            action.attributes.insert(
                attrs::mcp::EXECUTION_STATUS.to_string(),
                SemanticActionStatus::Error.as_str().to_string(),
            );
            action.attributes.insert(
                attrs::actrail::ACTION_FINALIZED_ON_TRACE_CLOSE.to_string(),
                "true".to_string(),
            );
            self.apply_parent_attributes(&key.server, &mut action.attributes);
            if let Some(action) = self.record_projected_action(action) {
                finalized.push(action);
            }
        }
        self.forget_trace(trace_id);
        finalized
    }

    pub(super) fn forget_trace(&mut self, trace_id: TraceId) {
        self.streams
            .retain(|key, _| key.server.trace_id != trace_id);
        self.http_streams.retain(|key, _| key.trace_id != trace_id);
        self.servers.retain(|key, _| key.trace_id != trace_id);
        self.open_calls
            .retain(|key, _| key.server.trace_id != trace_id);
        self.active_tool_calls
            .retain(|key, _| key.server.trace_id != trace_id);
        self.pending_http_requests
            .retain(|key, _| key.trace_id != trace_id);
        self.action_versions
            .retain(|(candidate, _), _| *candidate != trace_id);
        self.llm_tool_proposals
            .retain(|candidate, _| *candidate != trace_id);
        self.parents.retain(|key, _| key.trace_id != trace_id);
        self.completed_without_parent
            .retain(|key, _| key.trace_id != trace_id);
    }

    fn observe_message(
        &mut self,
        server_key: &McpServerKey,
        direction: McpStreamDirection,
        stream_key: &str,
        message: McpJsonMessage,
    ) -> Vec<SemanticAction> {
        if direction == McpStreamDirection::Outbound {
            self.observe_server_metadata(server_key, &message.value);
        }

        if direction == McpStreamDirection::Inbound
            && jsonrpc_method(&message.value) == Some("tools/call")
            && let Some(request_id) = jsonrpc_id(&message.value)
            && let Some(tool_name) = tools_call_name(&message.value)
        {
            return self.observe_tools_call_request(
                server_key,
                stream_key,
                &request_id,
                &tool_name,
                message,
            );
        }

        if direction == McpStreamDirection::Outbound
            && let Some(request_id) = jsonrpc_id(&message.value)
            && let Some(status) = jsonrpc_response_status(&message.value)
        {
            let request_key = McpRequestKey {
                server: server_key.clone(),
                request_id: request_id.clone(),
            };
            if let Some(action) = self.open_calls.remove(&request_key) {
                self.active_tool_calls.remove(&McpActiveToolCallKey {
                    server: server_key.clone(),
                    stream_pair_key: mcp_stream_pair_key(stream_key),
                });
                return self.complete_stdio_tool_response(
                    server_key,
                    action,
                    &request_id,
                    &message,
                    status,
                );
            }

            if is_mcp_tool_result_response(&message.value)
                && let Some(tool_name) = self.unique_tool_name(server_key)
            {
                let mut action = self.tool_call_action(
                    server_key,
                    stream_key,
                    &request_id,
                    &tool_name,
                    message.observed_at,
                    message.evidence.clone(),
                    SemanticActionCompleteness::Inferred,
                );
                action.status = status;
                action.end_time = Some(message.observed_at);
                action.attributes.insert(
                    attrs::mcp::EXECUTION_STATUS.to_string(),
                    status.as_str().to_string(),
                );
                action.attributes.insert(
                    attrs::mcp::EVIDENCE_MODE.to_string(),
                    MCP_EVIDENCE_MODE_RESPONSE_INFERRED_TOOL.to_string(),
                );
                return self.complete_stdio_tool_response(
                    server_key,
                    action,
                    &request_id,
                    &message,
                    status,
                );
            }
        }

        self.observe_active_stdio_message(server_key, stream_key, direction, message)
    }

    fn complete_stdio_tool_response(
        &mut self,
        server_key: &McpServerKey,
        mut action: SemanticAction,
        request_id: &str,
        message: &McpJsonMessage,
        status: SemanticActionStatus,
    ) -> Vec<SemanticAction> {
        action.status = status;
        action.completeness = SemanticActionCompleteness::Complete;
        action.end_time = Some(message.observed_at);
        action.attributes.insert(
            attrs::mcp::EXECUTION_STATUS.to_string(),
            status.as_str().to_string(),
        );
        self.apply_parent_attributes(server_key, &mut action.attributes);
        append_missing_evidence(&mut action.evidence, &message.evidence);
        self.insert_response_child_attributes(&mut action);
        let response = self.mcp_child_action(
            &action,
            SemanticActionKind::McpResponse,
            "response",
            "MCP response",
            message.observed_at,
            evidence_with_role(&message.evidence, evidence_roles::mcp::RESPONSE_PAYLOAD),
            status,
        );
        let mut stdin = self.mcp_child_action(
            &action,
            SemanticActionKind::McpStdin,
            "stdin",
            "MCP stdin",
            message.observed_at,
            evidence_with_role(&message.evidence, evidence_roles::mcp::STDIN_PAYLOAD),
            status,
        );
        insert_stdio_message_attributes(
            &mut stdin,
            &message.value,
            McpMessageDirection::Inbound,
            1,
            1,
            request_id,
        );
        self.record_mcp_actions(server_key, [action, response, stdin])
    }

    fn observe_http_payload_segment(&mut self, segment: &PayloadSegment) -> Vec<SemanticAction> {
        let stream_key = McpHttpStreamKey::from_segment(segment);
        let parse_buffer_max_bytes = self.parse_buffer_max_bytes;
        let messages = self
            .http_streams
            .entry(stream_key.clone())
            .or_insert_with(|| McpHttpStreamState::new(parse_buffer_max_bytes))
            .observe_segment(&stream_key, segment);
        let mut actions = Vec::new();
        for message in messages {
            match message {
                McpHttpMessage::Request(request) => {
                    actions.extend(self.observe_http_request(segment, &stream_key, request));
                }
                McpHttpMessage::Response(response) => {
                    actions.extend(self.observe_http_response(&stream_key, response));
                }
            }
        }
        actions
    }

    fn observe_http_request(
        &mut self,
        segment: &PayloadSegment,
        stream_key: &McpHttpStreamKey,
        request: McpHttpRequestMessage,
    ) -> Vec<SemanticAction> {
        if request
            .http
            .method
            .as_deref()
            .is_some_and(|method| method != "POST")
        {
            return Vec::new();
        }
        let server_key = McpServerKey::streamable_http_from_request(segment, &request.http);
        let exchange_key = stream_key.exchange_key(request.http.stream_id);
        let mut actions = Vec::new();
        for value in request.messages {
            let method = jsonrpc_method(&value);
            let request_id = jsonrpc_id(&value);
            if method.is_some() && request_id.is_some() {
                self.remember_http_request_context(
                    exchange_key.clone(),
                    McpHttpRequestContext {
                        request_id: request_id.clone(),
                        server_key: server_key.clone(),
                    },
                );
            }
            if method == Some("tools/call")
                && let Some(request_id) = request_id
                && let Some(tool_name) = tools_call_name(&value)
            {
                let message = McpJsonMessage {
                    value,
                    observed_at: request.observed_at,
                    evidence: request.evidence.clone(),
                };
                actions.extend(self.observe_remote_tools_call_request(
                    &server_key,
                    &request.http,
                    &request.source_boundary,
                    stream_key.stream_key.as_str(),
                    &request_id,
                    &tool_name,
                    message,
                ));
            } else {
                actions.extend(self.observe_active_remote_message(
                    &server_key,
                    stream_key.stream_key.as_str(),
                    &value,
                    request.observed_at,
                    &request.evidence,
                    None,
                    SemanticActionKind::McpClientSend,
                    evidence_roles::mcp::CLIENT_SEND_PAYLOAD,
                    McpMessageDirection::Outbound,
                ));
            }
        }
        actions
    }

    fn observe_http_response(
        &mut self,
        stream_key: &McpHttpStreamKey,
        response: McpHttpResponseMessage,
    ) -> Vec<SemanticAction> {
        let exchange_key = stream_key.exchange_key(response.http.stream_id);
        let response_session_id = response
            .http
            .headers_text
            .as_deref()
            .and_then(|headers| http_header_value(headers, "MCP-Session-Id"))
            .map(ToString::to_string);
        let mut actions = Vec::new();
        for value in response.messages {
            if jsonrpc_method(&value).is_some() {
                if let Some(server_key) = self.unique_active_remote_server_key(stream_key) {
                    actions.extend(self.observe_active_remote_message(
                        &server_key,
                        stream_key.stream_key.as_str(),
                        &value,
                        response.observed_at,
                        &response.evidence,
                        Some(&response.http),
                        SemanticActionKind::McpClientReceive,
                        evidence_roles::mcp::CLIENT_RECEIVE_PAYLOAD,
                        McpMessageDirection::Inbound,
                    ));
                }
                continue;
            }
            let Some(request_id) = jsonrpc_id(&value) else {
                continue;
            };
            let Some(context) = self.take_http_request_context(&exchange_key, &request_id) else {
                continue;
            };
            let metadata_server_key =
                context
                    .server_key
                    .with_http_session_id(response_session_id.clone().or_else(|| {
                        http_session_id_from_server_key(&context.server_key)
                            .map(ToString::to_string)
                    }));
            self.observe_server_metadata(&metadata_server_key, &value);
            if metadata_server_key != context.server_key {
                self.copy_server_metadata(&context.server_key, &metadata_server_key);
            }
            let Some(status) = jsonrpc_response_status(&value) else {
                continue;
            };
            let request_key = McpRequestKey {
                server: context.server_key.clone(),
                request_id: request_id.clone(),
            };
            if let Some(mut action) = self.open_calls.remove(&request_key) {
                self.active_tool_calls.remove(&McpActiveToolCallKey {
                    server: context.server_key.clone(),
                    stream_pair_key: mcp_stream_pair_key(&stream_key.stream_key),
                });
                action.status = status;
                action.completeness = SemanticActionCompleteness::Complete;
                action.end_time = Some(response.observed_at);
                action.attributes.insert(
                    attrs::mcp::EXECUTION_STATUS.to_string(),
                    status.as_str().to_string(),
                );
                append_missing_evidence(&mut action.evidence, &response.evidence);
                self.insert_response_child_attributes(&mut action);
                let mut response_action = self.mcp_child_action(
                    &action,
                    SemanticActionKind::McpResponse,
                    "response",
                    "MCP response",
                    response.observed_at,
                    evidence_with_role(&response.evidence, evidence_roles::mcp::RESPONSE_PAYLOAD),
                    status,
                );
                insert_http_response_attributes(&mut response_action.attributes, &response.http);
                let mut client_receive = self.mcp_child_action(
                    &action,
                    SemanticActionKind::McpClientReceive,
                    "client_receive",
                    "MCP client receive",
                    response.observed_at,
                    evidence_with_role(
                        &response.evidence,
                        evidence_roles::mcp::CLIENT_RECEIVE_PAYLOAD,
                    ),
                    status,
                );
                insert_http_response_attributes(&mut client_receive.attributes, &response.http);
                insert_stdio_message_attributes(
                    &mut client_receive,
                    &value,
                    McpMessageDirection::Inbound,
                    1,
                    1,
                    &request_id,
                );
                actions.extend(self.record_mcp_actions(
                    &context.server_key,
                    [action, response_action, client_receive],
                ));
            }
        }
        actions
    }

    fn observe_tools_call_request(
        &mut self,
        server_key: &McpServerKey,
        stream_key: &str,
        request_id: &str,
        tool_name: &str,
        message: McpJsonMessage,
    ) -> Vec<SemanticAction> {
        let mut action = self.tool_call_action(
            server_key,
            stream_key,
            request_id,
            tool_name,
            message.observed_at,
            message.evidence.clone(),
            SemanticActionCompleteness::Partial,
        );
        self.insert_request_child_attributes(&mut action);
        let request = self.mcp_child_action(
            &action,
            SemanticActionKind::McpRequest,
            "request",
            "MCP request",
            message.observed_at,
            evidence_with_role(&message.evidence, evidence_roles::mcp::REQUEST_PAYLOAD),
            SemanticActionStatus::Success,
        );
        let mut stdout = self.mcp_child_action(
            &action,
            SemanticActionKind::McpStdout,
            "stdout",
            "MCP stdout",
            message.observed_at,
            evidence_with_role(&message.evidence, evidence_roles::mcp::STDOUT_PAYLOAD),
            SemanticActionStatus::Success,
        );
        insert_stdio_message_attributes(
            &mut stdout,
            &message.value,
            McpMessageDirection::Outbound,
            1,
            1,
            request_id,
        );
        let request_key = McpRequestKey {
            server: server_key.clone(),
            request_id: request_id.to_string(),
        };
        self.open_calls.insert(request_key.clone(), action.clone());
        self.active_tool_calls.insert(
            McpActiveToolCallKey {
                server: server_key.clone(),
                stream_pair_key: mcp_stream_pair_key(stream_key),
            },
            McpActiveToolCall {
                request_key: Some(request_key),
                next_exchange_index: 2,
                pending_exchanges: BTreeMap::new(),
            },
        );
        self.record_mcp_actions(server_key, [action, request, stdout])
    }

    fn observe_remote_tools_call_request(
        &mut self,
        server_key: &McpServerKey,
        http: &HttpRequestParts,
        source_boundary: &str,
        stream_key: &str,
        request_id: &str,
        tool_name: &str,
        message: McpJsonMessage,
    ) -> Vec<SemanticAction> {
        let mut action = self.remote_tool_call_action(
            server_key,
            http,
            source_boundary,
            stream_key,
            request_id,
            tool_name,
            message.observed_at,
            message.evidence.clone(),
        );
        self.insert_request_child_attributes(&mut action);
        let request = self.mcp_child_action(
            &action,
            SemanticActionKind::McpRequest,
            "request",
            "MCP request",
            message.observed_at,
            evidence_with_role(&message.evidence, evidence_roles::mcp::REQUEST_PAYLOAD),
            SemanticActionStatus::Success,
        );
        let mut client_send = self.mcp_child_action(
            &action,
            SemanticActionKind::McpClientSend,
            "client_send",
            "MCP client send",
            message.observed_at,
            evidence_with_role(&message.evidence, evidence_roles::mcp::CLIENT_SEND_PAYLOAD),
            SemanticActionStatus::Success,
        );
        insert_stdio_message_attributes(
            &mut client_send,
            &message.value,
            McpMessageDirection::Outbound,
            1,
            1,
            request_id,
        );
        let request_key = McpRequestKey {
            server: server_key.clone(),
            request_id: request_id.to_string(),
        };
        self.open_calls.insert(request_key.clone(), action.clone());
        self.active_tool_calls.insert(
            McpActiveToolCallKey {
                server: server_key.clone(),
                stream_pair_key: mcp_stream_pair_key(stream_key),
            },
            McpActiveToolCall {
                request_key: Some(request_key),
                next_exchange_index: 2,
                pending_exchanges: BTreeMap::new(),
            },
        );
        self.record_mcp_actions(server_key, [action, request, client_send])
    }

    fn observe_active_remote_message(
        &mut self,
        server_key: &McpServerKey,
        stream_key: &str,
        value: &Value,
        observed_at: SystemTime,
        evidence: &[SemanticEvidence],
        http_response: Option<&HttpResponseParts>,
        kind: SemanticActionKind,
        evidence_role: &str,
        message_direction: McpMessageDirection,
    ) -> Vec<SemanticAction> {
        let Some((tool_call, exchange_index, tool_call_request_id)) =
            self.remote_active_message_context(server_key, stream_key, value)
        else {
            return Vec::new();
        };
        let suffix = match kind {
            SemanticActionKind::McpClientSend => format!("client_send:{exchange_index}"),
            SemanticActionKind::McpClientReceive => format!("client_receive:{exchange_index}"),
            _ => return Vec::new(),
        };
        let title_prefix = match kind {
            SemanticActionKind::McpClientSend => "MCP client send",
            SemanticActionKind::McpClientReceive => "MCP client receive",
            _ => return Vec::new(),
        };
        let mut action = self.mcp_child_action(
            &tool_call,
            kind,
            &suffix,
            title_prefix,
            observed_at,
            evidence_with_role(evidence, evidence_role),
            SemanticActionStatus::Success,
        );
        if let Some(http_response) = http_response {
            insert_http_response_attributes(&mut action.attributes, http_response);
        }
        action.attributes.insert(
            attrs::payload::STREAM_KEY.to_string(),
            stream_key.to_string(),
        );
        insert_stdio_message_attributes(
            &mut action,
            value,
            message_direction,
            exchange_index,
            exchange_index,
            &tool_call_request_id,
        );
        self.record_mcp_action(server_key, action)
            .into_iter()
            .collect()
    }

    fn remote_active_message_context(
        &mut self,
        server_key: &McpServerKey,
        stream_key: &str,
        value: &Value,
    ) -> Option<(SemanticAction, u64, String)> {
        let active_key = McpActiveToolCallKey {
            server: server_key.clone(),
            stream_pair_key: mcp_stream_pair_key(stream_key),
        };
        let active_key = if self.active_tool_calls.contains_key(&active_key) {
            active_key
        } else {
            self.remote_active_response_key(server_key, value)?
        };
        let active = self.active_tool_calls.get_mut(&active_key)?;
        let request_key = active.request_key.clone()?;
        let tool_call = self.open_calls.get(&request_key)?.clone();
        let exchange_index = match jsonrpc_id(value) {
            Some(message_id) if jsonrpc_method(value).is_some() => {
                let index = active.next_exchange_index;
                active.next_exchange_index += 1;
                active.pending_exchanges.insert(message_id, index);
                index
            }
            Some(message_id) => {
                if let Some(index) = active.pending_exchanges.remove(&message_id) {
                    index
                } else {
                    let index = active.next_exchange_index;
                    active.next_exchange_index += 1;
                    index
                }
            }
            None => {
                let index = active.next_exchange_index;
                active.next_exchange_index += 1;
                index
            }
        };
        Some((tool_call, exchange_index, request_key.request_id))
    }

    fn remote_active_response_key(
        &self,
        server_key: &McpServerKey,
        value: &Value,
    ) -> Option<McpActiveToolCallKey> {
        if jsonrpc_method(value).is_some() {
            return None;
        }
        let message_id = jsonrpc_id(value)?;
        self.active_tool_calls.iter().find_map(|(key, active)| {
            if &key.server != server_key || !active.pending_exchanges.contains_key(&message_id) {
                return None;
            }
            let request_key = active.request_key.as_ref()?;
            self.open_calls
                .contains_key(request_key)
                .then(|| key.clone())
        })
    }

    fn observe_active_stdio_message(
        &mut self,
        server_key: &McpServerKey,
        stream_key: &str,
        direction: McpStreamDirection,
        message: McpJsonMessage,
    ) -> Vec<SemanticAction> {
        let active_key = McpActiveToolCallKey {
            server: server_key.clone(),
            stream_pair_key: mcp_stream_pair_key(stream_key),
        };
        let Some((tool_call, exchange_index, tool_call_request_id)) =
            self.active_stdio_context(&active_key, &message.value, direction)
        else {
            return Vec::new();
        };
        let (kind, suffix, title_prefix, evidence_role, message_direction) = match direction {
            McpStreamDirection::Inbound => (
                SemanticActionKind::McpStdout,
                format!("stdout:{exchange_index}"),
                "MCP stdout",
                evidence_roles::mcp::STDOUT_PAYLOAD,
                McpMessageDirection::Outbound,
            ),
            McpStreamDirection::Outbound => (
                SemanticActionKind::McpStdin,
                format!("stdin:{exchange_index}"),
                "MCP stdin",
                evidence_roles::mcp::STDIN_PAYLOAD,
                McpMessageDirection::Inbound,
            ),
        };
        let mut stdio = self.mcp_child_action(
            &tool_call,
            kind,
            &suffix,
            title_prefix,
            message.observed_at,
            evidence_with_role(&message.evidence, evidence_role),
            SemanticActionStatus::Success,
        );
        insert_stdio_message_attributes(
            &mut stdio,
            &message.value,
            message_direction,
            exchange_index,
            exchange_index,
            &tool_call_request_id,
        );
        self.record_mcp_action(server_key, stdio)
            .into_iter()
            .collect()
    }

    fn active_stdio_context(
        &mut self,
        active_key: &McpActiveToolCallKey,
        value: &Value,
        direction: McpStreamDirection,
    ) -> Option<(SemanticAction, u64, String)> {
        let active = self.active_tool_calls.get_mut(active_key)?;
        let request_key = active.request_key.clone()?;
        let tool_call = self.open_calls.get(&request_key)?.clone();
        let exchange_index = match (direction, jsonrpc_id(value)) {
            (McpStreamDirection::Outbound, Some(message_id)) if jsonrpc_method(value).is_some() => {
                let index = active.next_exchange_index;
                active.next_exchange_index += 1;
                active.pending_exchanges.insert(message_id, index);
                index
            }
            (_, Some(message_id)) => {
                if let Some(index) = active.pending_exchanges.remove(&message_id) {
                    index
                } else {
                    let index = active.next_exchange_index;
                    active.next_exchange_index += 1;
                    index
                }
            }
            (_, None) => {
                let index = active.next_exchange_index;
                active.next_exchange_index += 1;
                index
            }
        };
        Some((tool_call, exchange_index, request_key.request_id))
    }

    fn tool_call_action(
        &self,
        server_key: &McpServerKey,
        stream_key: &str,
        request_id: &str,
        tool_name: &str,
        observed_at: SystemTime,
        evidence: Vec<SemanticEvidence>,
        completeness: SemanticActionCompleteness,
    ) -> SemanticAction {
        let mut attributes = BTreeMap::new();
        if let Some(server_name) = self.server_name(server_key) {
            attributes.insert(attrs::mcp::SERVER_NAME.to_string(), server_name);
        }
        attributes.insert(attrs::mcp::TOOL_NAME.to_string(), tool_name.to_string());
        attributes.insert(attrs::mcp::TOOL_ID.to_string(), tool_name.to_string());
        attributes.insert(attrs::mcp::REQUEST_ID.to_string(), request_id.to_string());
        attributes.insert(
            attrs::mcp::EXECUTION_STATUS.to_string(),
            SemanticActionStatus::InProgress.as_str().to_string(),
        );
        attributes.insert(
            attrs::mcp::TRANSPORT.to_string(),
            MCP_TRANSPORT_STDIO.to_string(),
        );
        attributes.insert(
            attrs::payload::SOURCE_BOUNDARY.to_string(),
            MCP_TRANSPORT_STDIO.to_string(),
        );
        attributes.insert(
            attrs::payload::STREAM_KEY.to_string(),
            stream_key.to_string(),
        );
        self.apply_parent_attributes(server_key, &mut attributes);
        self.apply_llm_proposal_attributes(server_key, tool_name, &mut attributes);

        SemanticAction {
            action_id: mcp_tool_action_id(server_key, stream_key, request_id),
            trace_id: server_key.trace_id,
            kind: SemanticActionKind::McpToolCall,
            title: format!("MCP tool {tool_name}"),
            start_time: observed_at,
            end_time: None,
            process: server_key.process.clone(),
            status: SemanticActionStatus::InProgress,
            completeness,
            confidence_millis: None,
            attributes,
            evidence,
        }
    }

    fn remote_tool_call_action(
        &self,
        server_key: &McpServerKey,
        http: &HttpRequestParts,
        source_boundary: &str,
        stream_key: &str,
        request_id: &str,
        tool_name: &str,
        observed_at: SystemTime,
        evidence: Vec<SemanticEvidence>,
    ) -> SemanticAction {
        let mut attributes = BTreeMap::new();
        if let Some(server_name) = self.server_name(server_key) {
            attributes.insert(attrs::mcp::SERVER_NAME.to_string(), server_name);
        }
        attributes.insert(attrs::mcp::TOOL_NAME.to_string(), tool_name.to_string());
        attributes.insert(attrs::mcp::TOOL_ID.to_string(), tool_name.to_string());
        attributes.insert(attrs::mcp::REQUEST_ID.to_string(), request_id.to_string());
        attributes.insert(
            attrs::mcp::EXECUTION_STATUS.to_string(),
            SemanticActionStatus::InProgress.as_str().to_string(),
        );
        attributes.insert(
            attrs::mcp::TRANSPORT.to_string(),
            MCP_TRANSPORT_STREAMABLE_HTTP.to_string(),
        );
        attributes.insert(
            attrs::mcp::CLIENT_PID.to_string(),
            server_key.process.pid.to_string(),
        );
        attributes.insert(
            attrs::payload::SOURCE_BOUNDARY.to_string(),
            source_boundary.to_string(),
        );
        attributes.insert(
            attrs::payload::STREAM_KEY.to_string(),
            stream_key.to_string(),
        );
        attributes.insert(
            attrs::http_request::PROTOCOL.to_string(),
            http.protocol.to_string(),
        );
        attributes.insert(
            attrs::network::PROTOCOL_NAME.to_string(),
            "http".to_string(),
        );
        attributes.insert(
            attrs::network::PROTOCOL_VERSION.to_string(),
            http.protocol.to_string(),
        );
        attributes.insert(
            attrs::url::SCHEME.to_string(),
            plaintext_http_scheme(source_boundary).to_string(),
        );
        if let Some(method) = &http.method {
            attributes.insert(attrs::http_request::METHOD.to_string(), method.clone());
        }
        if let Some(authority) = &http.authority {
            attributes.insert(attrs::server::ADDRESS.to_string(), authority.clone());
        }
        if let Some(path) = &http.path {
            attributes.insert(attrs::url::PATH.to_string(), path.clone());
        }
        if let Some(stream_id) = http.stream_id {
            attributes.insert(
                attrs::http_request::STREAM_ID.to_string(),
                stream_id.to_string(),
            );
        }
        if let Some(session_id) = http_session_id_from_server_key(server_key) {
            attributes.insert(attrs::mcp::SESSION_ID.to_string(), session_id.to_string());
        }
        self.apply_llm_proposal_attributes(server_key, tool_name, &mut attributes);

        SemanticAction {
            action_id: mcp_tool_action_id(server_key, stream_key, request_id),
            trace_id: server_key.trace_id,
            kind: SemanticActionKind::McpToolCall,
            title: format!("MCP tool {tool_name}"),
            start_time: observed_at,
            end_time: None,
            process: server_key.process.clone(),
            status: SemanticActionStatus::InProgress,
            completeness: SemanticActionCompleteness::Partial,
            confidence_millis: None,
            attributes,
            evidence,
        }
    }

    fn mcp_child_action(
        &self,
        tool_call: &SemanticAction,
        kind: SemanticActionKind,
        suffix: &str,
        title_prefix: &str,
        observed_at: SystemTime,
        evidence: Vec<SemanticEvidence>,
        status: SemanticActionStatus,
    ) -> SemanticAction {
        let mut attributes = tool_call.attributes.clone();
        attributes.insert(
            attrs::mcp::TOOL_CALL_ACTION_ID.to_string(),
            tool_call.action_id.clone(),
        );
        let action_id = mcp_child_action_id(&tool_call.action_id, suffix);
        if mcp_action_uses_stdio(tool_call) {
            match kind {
                SemanticActionKind::McpRequest => {
                    attributes.insert(
                        attrs::mcp::STDOUT_ACTION_ID.to_string(),
                        mcp_child_action_id(&tool_call.action_id, "stdout"),
                    );
                }
                SemanticActionKind::McpResponse => {
                    attributes.insert(
                        attrs::mcp::STDIN_ACTION_ID.to_string(),
                        mcp_child_action_id(&tool_call.action_id, "stdin"),
                    );
                }
                SemanticActionKind::McpStdin => {
                    attributes.insert(
                        attrs::mcp::RESPONSE_ACTION_ID.to_string(),
                        mcp_child_action_id(&tool_call.action_id, "response"),
                    );
                }
                SemanticActionKind::McpStdout => {
                    attributes.insert(
                        attrs::mcp::REQUEST_ACTION_ID.to_string(),
                        mcp_child_action_id(&tool_call.action_id, "request"),
                    );
                }
                _ => {}
            }
        } else {
            match kind {
                SemanticActionKind::McpRequest => {
                    attributes.insert(
                        attrs::mcp::CLIENT_SEND_ACTION_ID.to_string(),
                        mcp_child_action_id(&tool_call.action_id, "client_send"),
                    );
                }
                SemanticActionKind::McpResponse => {
                    attributes.insert(
                        attrs::mcp::CLIENT_RECEIVE_ACTION_ID.to_string(),
                        mcp_child_action_id(&tool_call.action_id, "client_receive"),
                    );
                }
                SemanticActionKind::McpClientSend => {
                    attributes.insert(
                        attrs::mcp::REQUEST_ACTION_ID.to_string(),
                        mcp_child_action_id(&tool_call.action_id, "request"),
                    );
                }
                SemanticActionKind::McpClientReceive => {
                    attributes.insert(
                        attrs::mcp::RESPONSE_ACTION_ID.to_string(),
                        mcp_child_action_id(&tool_call.action_id, "response"),
                    );
                }
                _ => {}
            }
        }
        let target = tool_call
            .attributes
            .get(attrs::mcp::TOOL_NAME)
            .map(String::as_str)
            .unwrap_or("tool");
        SemanticAction {
            action_id,
            trace_id: tool_call.trace_id,
            kind,
            title: format!("{title_prefix} {target}"),
            start_time: observed_at,
            end_time: Some(observed_at),
            process: tool_call.process.clone(),
            status,
            completeness: SemanticActionCompleteness::Complete,
            confidence_millis: None,
            attributes,
            evidence,
        }
    }

    fn insert_request_child_attributes(&self, action: &mut SemanticAction) {
        action.attributes.insert(
            attrs::mcp::REQUEST_ACTION_ID.to_string(),
            mcp_child_action_id(&action.action_id, "request"),
        );
        if mcp_action_uses_stdio(action) {
            action.attributes.insert(
                attrs::mcp::STDOUT_ACTION_ID.to_string(),
                mcp_child_action_id(&action.action_id, "stdout"),
            );
        } else {
            action.attributes.insert(
                attrs::mcp::CLIENT_SEND_ACTION_ID.to_string(),
                mcp_child_action_id(&action.action_id, "client_send"),
            );
        }
    }

    fn insert_response_child_attributes(&self, action: &mut SemanticAction) {
        action.attributes.insert(
            attrs::mcp::RESPONSE_ACTION_ID.to_string(),
            mcp_child_action_id(&action.action_id, "response"),
        );
        if mcp_action_uses_stdio(action) {
            action.attributes.insert(
                attrs::mcp::STDIN_ACTION_ID.to_string(),
                mcp_child_action_id(&action.action_id, "stdin"),
            );
        } else {
            action.attributes.insert(
                attrs::mcp::CLIENT_RECEIVE_ACTION_ID.to_string(),
                mcp_child_action_id(&action.action_id, "client_receive"),
            );
        }
    }

    fn observe_server_metadata(&mut self, server_key: &McpServerKey, value: &Value) {
        let state = self.servers.entry(server_key.clone()).or_default();
        if let Some(name) = value
            .pointer("/result/serverInfo/name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
        {
            state.name = Some(name.to_string());
        }
        let Some(tools) = value.pointer("/result/tools").and_then(Value::as_array) else {
            return;
        };
        state.tools = tools
            .iter()
            .filter_map(|tool| tool.get("name").and_then(Value::as_str))
            .filter(|name| !name.is_empty())
            .map(ToString::to_string)
            .collect();
    }

    fn copy_server_metadata(&mut self, from: &McpServerKey, to: &McpServerKey) {
        let Some(state) = self.servers.get(from).cloned() else {
            return;
        };
        self.servers.entry(to.clone()).or_insert(state);
    }

    fn server_name(&self, server_key: &McpServerKey) -> Option<String> {
        self.servers.get(server_key)?.name.clone()
    }

    fn unique_tool_name(&self, server_key: &McpServerKey) -> Option<String> {
        let tools = &self.servers.get(server_key)?.tools;
        if tools.len() == 1 {
            tools.iter().next().cloned()
        } else {
            None
        }
    }

    fn apply_parent_attributes(
        &self,
        server_key: &McpServerKey,
        attributes: &mut BTreeMap<String, String>,
    ) {
        if !server_key.is_stdio() {
            return;
        }
        let parent_key = McpProcessKey::from_server_key(server_key);
        let Some(parent) = self.parents.get(&parent_key) else {
            return;
        };
        insert_parent_identity_attributes(attributes, parent);
        attributes.insert(attrs::mcp::CLIENT_PID.to_string(), parent.pid.to_string());
    }

    fn apply_llm_proposal_attributes(
        &self,
        server_key: &McpServerKey,
        tool_name: &str,
        attributes: &mut BTreeMap<String, String>,
    ) {
        let Some(server_name) = self.server_name(server_key) else {
            return;
        };
        let Some(proposal) =
            self.matching_llm_tool_proposal(server_key.trace_id, server_name.as_str(), tool_name)
        else {
            return;
        };
        attributes.insert(
            attrs::llm_response::ACTION_ID.to_string(),
            proposal.response_action_id.clone(),
        );
        if let Some(tool_call_id) = &proposal.tool_call_id {
            attributes.insert(attrs::llm_tool_call::ID.to_string(), tool_call_id.clone());
        }
        attributes.insert(
            attrs::llm_tool_call::NAME.to_string(),
            proposal.name.clone(),
        );
    }

    fn matching_llm_tool_proposal(
        &self,
        trace_id: TraceId,
        server_name: &str,
        tool_name: &str,
    ) -> Option<&LlmToolCallProposal> {
        self.llm_tool_proposals
            .get(&trace_id)?
            .iter()
            .rev()
            .find(|proposal| {
                proposal.server_name == server_name
                    && (proposal.tool_name == tool_name
                        || tool_name == format!("{}.{}", server_name, proposal.tool_name))
            })
    }

    fn apply_late_parent_identity(&mut self, process_key: &McpProcessKey) -> Vec<SemanticAction> {
        let Some(actions) = self.completed_without_parent.remove(process_key) else {
            return Vec::new();
        };
        let mut updated = Vec::new();
        for (_, mut action) in actions {
            let server_key = McpServerKey {
                trace_id: action.trace_id,
                process: action.process.clone(),
                endpoint: McpEndpointKey::Stdio,
            };
            self.apply_parent_attributes(&server_key, &mut action.attributes);
            if let Some(action) = self.record_projected_action(action) {
                updated.push(action);
            }
        }
        updated
    }

    fn record_mcp_action(
        &mut self,
        server_key: &McpServerKey,
        action: SemanticAction,
    ) -> Option<SemanticAction> {
        let parent_key = McpProcessKey::from_server_key(server_key);
        if server_key.is_stdio()
            && action.status != SemanticActionStatus::InProgress
            && !self.parents.contains_key(&parent_key)
        {
            self.completed_without_parent
                .entry(parent_key)
                .or_default()
                .insert(action.action_id.clone(), action.clone());
        }
        self.record_projected_action(action)
    }

    fn record_mcp_actions<const N: usize>(
        &mut self,
        server_key: &McpServerKey,
        actions: [SemanticAction; N],
    ) -> Vec<SemanticAction> {
        actions
            .into_iter()
            .filter_map(|action| self.record_mcp_action(server_key, action))
            .collect()
    }

    fn record_projected_action(&mut self, action: SemanticAction) -> Option<SemanticAction> {
        let key = (action.trace_id, action.action_id.clone());
        if self
            .action_versions
            .get(&key)
            .is_some_and(|existing| existing == &action)
        {
            return None;
        }
        if action.status == SemanticActionStatus::InProgress {
            self.action_versions.insert(key, action.clone());
        } else {
            self.action_versions.remove(&key);
        }
        Some(action)
    }

    fn remember_http_request_context(
        &mut self,
        exchange_key: McpHttpExchangeKey,
        context: McpHttpRequestContext,
    ) {
        let contexts = self.pending_http_requests.entry(exchange_key).or_default();
        contexts.push_back(context);
        while contexts.len() > MAX_PENDING_HTTP_REQUEST_CONTEXTS {
            contexts.pop_front();
        }
    }

    fn take_http_request_context(
        &mut self,
        exchange_key: &McpHttpExchangeKey,
        request_id: &str,
    ) -> Option<McpHttpRequestContext> {
        let contexts = self.pending_http_requests.get_mut(exchange_key)?;
        let index = contexts
            .iter()
            .position(|context| context.request_id.as_deref() == Some(request_id))?;
        let context = contexts.remove(index)?;
        if contexts.is_empty() {
            self.pending_http_requests.remove(exchange_key);
        }
        Some(context)
    }

    fn unique_active_remote_server_key(
        &self,
        stream_key: &McpHttpStreamKey,
    ) -> Option<McpServerKey> {
        let stream_pair_key = mcp_stream_pair_key(&stream_key.stream_key);
        let exact = self
            .active_remote_server_keys(stream_key)
            .filter(|key| {
                self.active_tool_calls.keys().any(|active_key| {
                    active_key.server == **key && active_key.stream_pair_key == stream_pair_key
                })
            })
            .collect::<BTreeSet<_>>();
        if exact.len() == 1 {
            return exact.into_iter().next().cloned();
        }

        let all = self
            .active_remote_server_keys(stream_key)
            .collect::<BTreeSet<_>>();
        (all.len() == 1)
            .then(|| all.into_iter().next().cloned())
            .flatten()
    }

    fn active_remote_server_keys<'a>(
        &'a self,
        stream_key: &'a McpHttpStreamKey,
    ) -> impl Iterator<Item = &'a McpServerKey> + 'a {
        self.active_tool_calls
            .iter()
            .filter_map(move |(key, active)| {
                if key.server.trace_id != stream_key.trace_id
                    || key.server.process != stream_key.process
                    || key.server.is_stdio()
                {
                    return None;
                }
                let request_key = active.request_key.as_ref()?;
                self.open_calls
                    .contains_key(request_key)
                    .then_some(&key.server)
            })
    }
}

fn insert_http_response_attributes(
    attributes: &mut BTreeMap<String, String>,
    http: &HttpResponseParts,
) {
    remove_http_request_attributes(attributes);
    attributes.insert(
        attrs::http_response::PROTOCOL.to_string(),
        http.protocol.to_string(),
    );
    if let Some(status_code) = &http.status_code {
        attributes.insert(
            attrs::http_response::STATUS_CODE.to_string(),
            status_code.clone(),
        );
    }
    if let Some(reason) = &http.reason {
        attributes.insert(attrs::http_response::REASON.to_string(), reason.clone());
    }
    if let Some(stream_id) = http.stream_id {
        attributes.insert(
            attrs::http_response::STREAM_ID.to_string(),
            stream_id.to_string(),
        );
    }
    if let Some(headers_text) = &http.headers_text {
        attributes.insert(
            attrs::http_response::HEADERS_TEXT.to_string(),
            headers_text.clone(),
        );
        attributes.insert(
            attrs::http_response::HEADERS_ENCODING.to_string(),
            "text".to_string(),
        );
    }
    if let Some(headers_hpack_base64) = &http.headers_hpack_base64 {
        attributes.insert(
            attrs::http_response::HEADERS_HPACK_BASE64.to_string(),
            headers_hpack_base64.clone(),
        );
        attributes.insert(
            attrs::http_response::HEADERS_ENCODING.to_string(),
            "hpack".to_string(),
        );
    }
}

fn remove_http_request_attributes(attributes: &mut BTreeMap<String, String>) {
    for key in [
        attrs::http_request::METHOD,
        attrs::http_request::PROTOCOL,
        attrs::http_request::STREAM_ID,
        attrs::http_request::HEADERS_TEXT,
        attrs::http_request::HEADERS_ENCODING,
        attrs::http_request::HEADERS_HPACK_BASE64,
    ] {
        attributes.remove(key);
    }
}

struct McpStreamState {
    parse_buffer_max_bytes: usize,
    buffer: Vec<u8>,
    base_offset: usize,
    segments: VecDeque<McpSegmentRange>,
}

#[derive(Clone, Debug)]
struct McpSegmentRange {
    start: usize,
    end: usize,
    segment_id: u64,
    observed_at: SystemTime,
    source_boundary: PayloadSourceBoundary,
}

impl McpStreamState {
    fn new(parse_buffer_max_bytes: usize) -> Self {
        Self {
            parse_buffer_max_bytes,
            buffer: Vec::new(),
            base_offset: 0,
            segments: VecDeque::new(),
        }
    }

    fn observe_segment(&mut self, segment: &PayloadSegment) -> Vec<McpJsonMessage> {
        self.append_segment(segment);
        let messages = self.parse_messages();
        self.discard_oversized_incomplete_buffer();
        messages
    }

    fn append_segment(&mut self, segment: &PayloadSegment) {
        let start = self.base_offset + self.buffer.len();
        self.buffer.extend_from_slice(&segment.bytes);
        let end = self.base_offset + self.buffer.len();
        self.segments.push_back(McpSegmentRange {
            start,
            end,
            segment_id: segment.segment_id.get(),
            observed_at: segment.observed_at,
            source_boundary: segment.source_boundary,
        });
    }

    fn parse_messages(&mut self) -> Vec<McpJsonMessage> {
        let mut messages = Vec::new();
        loop {
            let leading_empty = leading_empty_line_len(&self.buffer);
            if leading_empty > 0 {
                self.evict_encoded_len(leading_empty);
                continue;
            }
            if self.buffer.is_empty() {
                break;
            }
            if starts_with_content_length(&self.buffer) {
                let Some((encoded_len, body_start, body_end)) = self.next_framed_message_range()
                else {
                    break;
                };
                let start = self.base_offset;
                let end = start + encoded_len;
                let value = serde_json::from_slice::<Value>(&self.buffer[body_start..body_end]);
                if let Ok(value) = value {
                    let (evidence, observed_at) = self.message_context(start, end);
                    messages.push(McpJsonMessage {
                        value,
                        observed_at,
                        evidence,
                    });
                }
                self.evict_encoded_len(encoded_len);
                continue;
            }

            let Some(line_len) = newline_message_len(&self.buffer) else {
                break;
            };
            let start = self.base_offset;
            let end = start + line_len;
            let line = String::from_utf8_lossy(&self.buffer[..line_len]);
            let trimmed = line.trim();
            if !trimmed.is_empty()
                && let Ok(value) = serde_json::from_str::<Value>(trimmed)
            {
                let (evidence, observed_at) = self.message_context(start, end);
                messages.push(McpJsonMessage {
                    value,
                    observed_at,
                    evidence,
                });
            }
            self.evict_encoded_len(line_len);
        }
        messages
    }

    fn next_framed_message_range(&self) -> Option<(usize, usize, usize)> {
        let header_end = find_subslice(&self.buffer, b"\r\n\r\n")?;
        let headers = String::from_utf8_lossy(&self.buffer[..header_end]);
        let content_length = headers.lines().find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("Content-Length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })?;
        let body_start = header_end + 4;
        let body_end = body_start.checked_add(content_length)?;
        if self.buffer.len() < body_end {
            return None;
        }
        Some((body_end, body_start, body_end))
    }

    fn message_context(&self, start: usize, end: usize) -> (Vec<SemanticEvidence>, SystemTime) {
        let mut observed_at = SystemTime::UNIX_EPOCH;
        let evidence = self
            .segments
            .iter()
            .filter(|segment| segment.end > start && segment.start < end)
            .map(|segment| {
                observed_at = observed_at.max(segment.observed_at);
                SemanticEvidence {
                    kind: SemanticEvidenceKind::PayloadSegment,
                    id: segment.segment_id,
                    role: evidence_roles::mcp::TOOL_CALL_PAYLOAD.to_string(),
                }
            })
            .collect();
        (evidence, observed_at)
    }

    fn evict_encoded_len(&mut self, encoded_len: usize) {
        self.buffer.drain(..encoded_len);
        self.base_offset += encoded_len;
        while self
            .segments
            .front()
            .is_some_and(|segment| segment.end <= self.base_offset)
        {
            self.segments.pop_front();
        }
    }

    fn discard_oversized_incomplete_buffer(&mut self) {
        if self.buffer.len() <= self.parse_buffer_max_bytes {
            return;
        }
        self.discard_buffered_bytes();
    }

    fn discard_buffered_bytes(&mut self) {
        let len = self.buffer.len();
        self.buffer.clear();
        self.base_offset = self.base_offset.saturating_add(len);
        self.segments.clear();
    }
}

struct McpHttpStreamState {
    parse_buffer_max_bytes: usize,
    buffer: Vec<u8>,
    base_offset: usize,
    segments: VecDeque<McpSegmentRange>,
    pending_sse_responses: BTreeMap<Option<u32>, McpPendingSseResponse>,
}

impl McpHttpStreamState {
    fn new(parse_buffer_max_bytes: usize) -> Self {
        Self {
            parse_buffer_max_bytes,
            buffer: Vec::new(),
            base_offset: 0,
            segments: VecDeque::new(),
            pending_sse_responses: BTreeMap::new(),
        }
    }

    fn observe_segment(
        &mut self,
        key: &McpHttpStreamKey,
        segment: &PayloadSegment,
    ) -> Vec<McpHttpMessage> {
        self.append_segment(segment);
        let messages = match key.direction {
            McpStreamDirection::Outbound => self.parse_requests(),
            McpStreamDirection::Inbound => self.parse_responses(),
        };
        self.discard_oversized_incomplete_buffer();
        messages
    }

    fn append_segment(&mut self, segment: &PayloadSegment) {
        let start = self.base_offset + self.buffer.len();
        self.buffer.extend_from_slice(&segment.bytes);
        let end = self.base_offset + self.buffer.len();
        self.segments.push_back(McpSegmentRange {
            start,
            end,
            segment_id: segment.segment_id.get(),
            observed_at: segment.observed_at,
            source_boundary: segment.source_boundary,
        });
    }

    fn parse_requests(&mut self) -> Vec<McpHttpMessage> {
        let mut messages = Vec::new();
        loop {
            let Some(batch) = split_request_batch(&self.buffer) else {
                break;
            };
            if batch.consumed_len == 0 {
                break;
            }
            let consumed_len = batch.consumed_len;
            for message in batch.messages {
                let start = self.base_offset + message.start;
                let end = self.base_offset + message.end;
                let http = message.parts;
                let jsonrpc_messages = jsonrpc_messages_from_body(&http.body);
                if jsonrpc_messages.is_empty() {
                    continue;
                }
                let (evidence, observed_at, source_boundary) = self.message_context(start, end);
                messages.push(McpHttpMessage::Request(McpHttpRequestMessage {
                    http,
                    messages: jsonrpc_messages,
                    observed_at,
                    source_boundary,
                    evidence,
                }));
            }
            self.evict_encoded_len(consumed_len);
            if self.buffer.is_empty() {
                break;
            }
        }
        messages
    }

    fn parse_responses(&mut self) -> Vec<McpHttpMessage> {
        let mut messages = Vec::new();
        'parse: loop {
            let (pending_messages, consumed, waiting_for_http1_sse) =
                self.parse_pending_http1_sse_response();
            messages.extend(pending_messages);
            if consumed {
                if self.buffer.is_empty() {
                    break;
                }
                continue;
            }
            if waiting_for_http1_sse {
                break;
            }

            if self.start_http1_chunked_sse_response() {
                if self.buffer.is_empty() {
                    break;
                }
                continue;
            }

            let Some(batch) = split_response_batch(&self.buffer) else {
                break;
            };
            if batch.consumed_len == 0 {
                break;
            }

            let consumed_len = batch.consumed_len;
            let mut wait_for_more_body = false;
            for message in batch.messages {
                let start = self.base_offset + message.start;
                let end = self.base_offset + message.end;
                let http = message.parts;
                if http.body_boundary_known && !http.complete {
                    wait_for_more_body = true;
                    break;
                }
                if is_http2_sse_response_candidate(&http)
                    || self.pending_sse_responses.contains_key(&http.stream_id)
                {
                    let body = http.body.clone();
                    let (evidence, observed_at, _) = self.message_context(start, end);
                    messages.extend(self.append_pending_sse_body(
                        http.stream_id,
                        http,
                        observed_at,
                        evidence,
                        body.as_slice(),
                        McpPendingSseMode::Accumulated,
                        false,
                    ));
                    continue;
                }
                if is_open_sse_response(&http) {
                    let Some(header_len) = http1_response_header_len(&self.buffer) else {
                        wait_for_more_body = true;
                        break;
                    };
                    let header_start = self.base_offset;
                    let header_end = header_start + header_len;
                    let (header_evidence, observed_at, _) =
                        self.message_context(header_start, header_end);
                    let mut pending_http = http;
                    pending_http.body.clear();
                    pending_http.encoded_len = header_len;
                    self.pending_sse_responses.insert(
                        None,
                        McpPendingSseResponse {
                            http: pending_http,
                            observed_at,
                            stream_evidence: header_evidence,
                            message_evidence: Vec::new(),
                            body: Vec::new(),
                            mode: McpPendingSseMode::RawHttp1,
                        },
                    );
                    self.evict_encoded_len(header_len);
                    if self.buffer.is_empty() {
                        break 'parse;
                    }
                    continue 'parse;
                }
                let jsonrpc_messages = jsonrpc_messages_from_http_response(&http);
                if jsonrpc_messages.is_empty() {
                    continue;
                }
                let (evidence, observed_at, _) = self.message_context(start, end);
                messages.push(McpHttpMessage::Response(McpHttpResponseMessage {
                    http,
                    messages: jsonrpc_messages,
                    observed_at,
                    evidence,
                }));
            }
            if wait_for_more_body {
                break;
            }
            self.evict_encoded_len(consumed_len);
            if self.buffer.is_empty() {
                break;
            }
        }
        messages
    }

    fn parse_pending_http1_sse_response(&mut self) -> (Vec<McpHttpMessage>, bool, bool) {
        let Some(mode) = self
            .pending_sse_responses
            .get(&None)
            .map(|pending| pending.mode)
        else {
            return (Vec::new(), false, false);
        };
        match mode {
            McpPendingSseMode::RawHttp1 => self.parse_pending_raw_http1_sse_response(),
            McpPendingSseMode::ChunkedHttp1 => self.parse_pending_chunked_http1_sse_response(),
            McpPendingSseMode::Accumulated => (Vec::new(), false, false),
        }
    }

    fn parse_pending_raw_http1_sse_response(&mut self) -> (Vec<McpHttpMessage>, bool, bool) {
        let Some((_, consumed_len)) = jsonrpc_messages_from_complete_sse_prefix(&self.buffer)
        else {
            return (Vec::new(), false, true);
        };
        let start = self.base_offset;
        let end = start + consumed_len;
        let body = self.buffer[..consumed_len].to_vec();
        let (body_evidence, body_observed_at, _) = self.message_context(start, end);
        if let Some(pending) = self.pending_sse_responses.get_mut(&None) {
            append_missing_evidence(&mut pending.message_evidence, &body_evidence);
            if body_observed_at != SystemTime::UNIX_EPOCH {
                pending.observed_at = body_observed_at;
            }
            pending.body.extend_from_slice(&body);
        }
        self.evict_encoded_len(consumed_len);
        let messages = self.emit_pending_sse_messages(None);
        (messages, true, false)
    }

    fn parse_pending_chunked_http1_sse_response(&mut self) -> (Vec<McpHttpMessage>, bool, bool) {
        let Some(chunked) = parse_complete_http1_chunks_prefix(&self.buffer) else {
            return (Vec::new(), false, true);
        };
        if chunked.encoded_len == 0 {
            return (Vec::new(), false, true);
        }
        let start = self.base_offset;
        let end = start + chunked.encoded_len;
        let (evidence, observed_at, _) = self.message_context(start, end);
        let messages = self.append_pending_sse_body(
            None,
            self.pending_sse_responses
                .get(&None)
                .expect("chunked SSE pending response should exist")
                .http
                .clone(),
            observed_at,
            evidence,
            &chunked.body,
            McpPendingSseMode::ChunkedHttp1,
            chunked.complete,
        );
        self.evict_encoded_len(chunked.encoded_len);
        (messages, true, false)
    }

    fn start_http1_chunked_sse_response(&mut self) -> bool {
        if self.pending_sse_responses.contains_key(&None) {
            return false;
        }
        let Some((http, header_len)) = http1_chunked_sse_response_prefix(&self.buffer) else {
            return false;
        };
        let header_start = self.base_offset;
        let header_end = header_start + header_len;
        let (header_evidence, observed_at, _) = self.message_context(header_start, header_end);
        self.pending_sse_responses.insert(
            None,
            McpPendingSseResponse {
                http,
                observed_at,
                stream_evidence: header_evidence,
                message_evidence: Vec::new(),
                body: Vec::new(),
                mode: McpPendingSseMode::ChunkedHttp1,
            },
        );
        self.evict_encoded_len(header_len);
        true
    }

    fn append_pending_sse_body(
        &mut self,
        stream_id: Option<u32>,
        http: HttpResponseParts,
        observed_at: SystemTime,
        evidence: Vec<SemanticEvidence>,
        body: &[u8],
        mode: McpPendingSseMode,
        complete: bool,
    ) -> Vec<McpHttpMessage> {
        self.pending_sse_responses
            .entry(stream_id)
            .or_insert_with(|| McpPendingSseResponse {
                http,
                observed_at,
                stream_evidence: Vec::new(),
                message_evidence: Vec::new(),
                body: Vec::new(),
                mode,
            });
        if let Some(pending) = self.pending_sse_responses.get_mut(&stream_id) {
            append_missing_evidence(&mut pending.message_evidence, &evidence);
            if observed_at != SystemTime::UNIX_EPOCH {
                pending.observed_at = observed_at;
            }
            pending.body.extend_from_slice(body);
        }
        let messages = self.emit_pending_sse_messages(stream_id);
        if complete
            && self
                .pending_sse_responses
                .get(&stream_id)
                .is_some_and(|pending| pending.body.is_empty())
        {
            self.pending_sse_responses.remove(&stream_id);
        }
        messages
    }

    fn emit_pending_sse_messages(&mut self, stream_id: Option<u32>) -> Vec<McpHttpMessage> {
        let Some((jsonrpc_messages, consumed_len)) = self
            .pending_sse_responses
            .get(&stream_id)
            .and_then(|pending| jsonrpc_messages_from_complete_sse_prefix(&pending.body))
        else {
            return Vec::new();
        };
        let Some(pending) = self.pending_sse_responses.get_mut(&stream_id) else {
            return Vec::new();
        };
        let body = pending.body[..consumed_len].to_vec();
        pending.body.drain(..consumed_len);
        let mut evidence = pending.stream_evidence.clone();
        append_missing_evidence(&mut evidence, &pending.message_evidence);
        if pending.body.is_empty() {
            pending.message_evidence.clear();
        }
        let mut http = pending.http.clone();
        http.body = body;
        http.encoded_len = consumed_len;
        if jsonrpc_messages.is_empty() {
            return Vec::new();
        }
        vec![McpHttpMessage::Response(McpHttpResponseMessage {
            http,
            messages: jsonrpc_messages,
            observed_at: pending.observed_at,
            evidence,
        })]
    }

    fn message_context(
        &self,
        start: usize,
        end: usize,
    ) -> (Vec<SemanticEvidence>, SystemTime, String) {
        let mut observed_at = SystemTime::UNIX_EPOCH;
        let mut source_boundary = None;
        let evidence = self
            .segments
            .iter()
            .filter(|segment| segment.end > start && segment.start < end)
            .map(|segment| {
                observed_at = observed_at.max(segment.observed_at);
                source_boundary.get_or_insert(segment.source_boundary);
                SemanticEvidence {
                    kind: SemanticEvidenceKind::PayloadSegment,
                    id: segment.segment_id,
                    role: evidence_roles::mcp::TOOL_CALL_PAYLOAD.to_string(),
                }
            })
            .collect();
        (
            evidence,
            observed_at,
            source_boundary
                .map(|boundary| format!("{boundary:?}"))
                .unwrap_or_else(|| "unknown".to_string()),
        )
    }

    fn evict_encoded_len(&mut self, encoded_len: usize) {
        self.buffer.drain(..encoded_len);
        self.base_offset += encoded_len;
        while self
            .segments
            .front()
            .is_some_and(|segment| segment.end <= self.base_offset)
        {
            self.segments.pop_front();
        }
    }

    fn discard_oversized_incomplete_buffer(&mut self) {
        if self.buffer.len() <= self.parse_buffer_max_bytes {
            return;
        }
        self.discard_buffered_bytes();
    }

    fn discard_buffered_bytes(&mut self) {
        let len = self.buffer.len();
        self.buffer.clear();
        self.base_offset = self.base_offset.saturating_add(len);
        self.segments.clear();
    }
}

fn jsonrpc_method(value: &Value) -> Option<&str> {
    value.get("method")?.as_str()
}

fn jsonrpc_id(value: &Value) -> Option<String> {
    match value.get("id")? {
        Value::String(id) if !id.is_empty() => Some(id.clone()),
        Value::Number(id) => Some(id.to_string()),
        _ => None,
    }
}

fn tools_call_name(value: &Value) -> Option<String> {
    value
        .pointer("/params/name")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
        .map(ToString::to_string)
}

fn jsonrpc_response_status(value: &Value) -> Option<SemanticActionStatus> {
    if value.get("error").is_some() {
        return Some(SemanticActionStatus::Error);
    }
    let result = value.get("result")?;
    if result
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        Some(SemanticActionStatus::Error)
    } else {
        Some(SemanticActionStatus::Success)
    }
}

fn is_mcp_tool_result_response(value: &Value) -> bool {
    value
        .get("result")
        .is_some_and(|result| result.get("content").is_some_and(Value::is_array))
}

fn plaintext_http_candidate(segment: &PayloadSegment) -> bool {
    matches!(
        segment.source_boundary,
        PayloadSourceBoundary::TlsUserSpace | PayloadSourceBoundary::Syscall
    ) && segment.content_state == PayloadContentState::Plaintext
}

fn jsonrpc_messages_from_http_response(http: &HttpResponseParts) -> Vec<Value> {
    let content_type = http
        .headers_text
        .as_deref()
        .and_then(|headers| http_header_value(headers, "content-type"));
    if content_type.is_some_and(|value| value.to_ascii_lowercase().contains("text/event-stream")) {
        let messages = jsonrpc_messages_from_sse_body(&http.body);
        if !messages.is_empty() {
            return messages;
        }
    }
    jsonrpc_messages_from_body(&http.body)
}

fn is_open_sse_response(http: &HttpResponseParts) -> bool {
    let content_type = http
        .headers_text
        .as_deref()
        .and_then(|headers| http_header_value(headers, "content-type"));
    content_type.is_some_and(|value| value.to_ascii_lowercase().contains("text/event-stream"))
        && !http.body_boundary_known
}

fn is_http2_sse_response_candidate(http: &HttpResponseParts) -> bool {
    http.protocol == "h2" && body_starts_like_sse(&http.body)
}

fn http1_chunked_sse_response_prefix(bytes: &[u8]) -> Option<(HttpResponseParts, usize)> {
    let header_len = http1_response_header_len(bytes)?;
    let header_text = String::from_utf8_lossy(&bytes[..header_len - b"\r\n\r\n".len()]);
    let content_type = http_header_value(&header_text, "content-type");
    if !content_type.is_some_and(|value| value.to_ascii_lowercase().contains("text/event-stream")) {
        return None;
    }
    let transfer_encoding = http_header_value(&header_text, "transfer-encoding");
    if !transfer_encoding.is_some_and(|value| value.to_ascii_lowercase().contains("chunked")) {
        return None;
    }
    let (protocol, status_code, reason) = parse_mcp_http1_status_line(&header_text)?;
    Some((
        HttpResponseParts {
            protocol,
            scheme: "https",
            status_code: Some(status_code),
            reason,
            stream_id: None,
            headers_text: Some(header_text.into_owned()),
            headers_hpack_base64: None,
            body: Vec::new(),
            encoded_len: header_len,
            complete: false,
            body_boundary_known: true,
        },
        header_len,
    ))
}

fn parse_mcp_http1_status_line(
    header_text: &str,
) -> Option<(&'static str, String, Option<String>)> {
    let first_line = header_text.lines().next()?.trim();
    if !first_line.starts_with("HTTP/") {
        return None;
    }
    let mut parts = first_line.splitn(3, ' ');
    let protocol = mcp_http1_response_protocol(parts.next()?)?;
    let status_code = parts.next()?.to_string();
    let reason = parts.next().map(ToString::to_string);
    Some((protocol, status_code, reason))
}

fn mcp_http1_response_protocol(version: &str) -> Option<&'static str> {
    match version {
        "HTTP/1.0" => Some("http/1.0"),
        "HTTP/1.1" => Some("http/1.1"),
        _ => None,
    }
}

fn http1_response_header_len(bytes: &[u8]) -> Option<usize> {
    bytes
        .starts_with(b"HTTP/")
        .then(|| find_subslice(bytes, b"\r\n\r\n").map(|index| index + b"\r\n\r\n".len()))?
}

fn jsonrpc_messages_from_body(body: &[u8]) -> Vec<Value> {
    let Ok(value) = serde_json::from_slice::<Value>(body) else {
        return Vec::new();
    };
    jsonrpc_messages_from_value(value)
}

struct Http1CompleteChunksPrefix {
    body: Vec<u8>,
    encoded_len: usize,
    complete: bool,
}

fn parse_complete_http1_chunks_prefix(bytes: &[u8]) -> Option<Http1CompleteChunksPrefix> {
    let mut cursor = 0;
    let mut body = Vec::new();
    let mut parsed_chunk = false;
    loop {
        if cursor >= bytes.len() {
            return parsed_chunk.then_some(Http1CompleteChunksPrefix {
                body,
                encoded_len: cursor,
                complete: false,
            });
        }
        let Some(line_end) = find_subslice(&bytes[cursor..], b"\r\n") else {
            return parsed_chunk.then_some(Http1CompleteChunksPrefix {
                body,
                encoded_len: cursor,
                complete: false,
            });
        };
        let size_line = &bytes[cursor..cursor + line_end];
        let size_text = std::str::from_utf8(size_line)
            .ok()?
            .split(';')
            .next()
            .unwrap_or_default()
            .trim();
        let size = usize::from_str_radix(size_text, 16).ok()?;
        cursor = cursor.checked_add(line_end)?.checked_add(b"\r\n".len())?;
        let data_end = cursor.checked_add(size)?;
        let chunk_end = data_end.checked_add(b"\r\n".len())?;
        if bytes.len() < chunk_end {
            return parsed_chunk.then_some(Http1CompleteChunksPrefix {
                body,
                encoded_len: cursor.saturating_sub(line_end + b"\r\n".len()),
                complete: false,
            });
        }
        if &bytes[data_end..chunk_end] != b"\r\n" {
            return None;
        }
        body.extend_from_slice(&bytes[cursor..data_end]);
        cursor = chunk_end;
        parsed_chunk = true;
        if size == 0 {
            return Some(Http1CompleteChunksPrefix {
                body,
                encoded_len: cursor,
                complete: true,
            });
        }
    }
}

fn jsonrpc_messages_from_complete_sse_prefix(bytes: &[u8]) -> Option<(Vec<Value>, usize)> {
    let mut cursor = 0;
    let mut consumed = None;
    while cursor < bytes.len() {
        let lf_delimiter =
            find_subslice(&bytes[cursor..], b"\n\n").map(|index| cursor + index + b"\n\n".len());
        let crlf_delimiter = find_subslice(&bytes[cursor..], b"\r\n\r\n")
            .map(|index| cursor + index + b"\r\n\r\n".len());
        let Some(next_consumed) = (match (lf_delimiter, crlf_delimiter) {
            (Some(lf), Some(crlf)) => Some(lf.min(crlf)),
            (Some(lf), None) => Some(lf),
            (None, Some(crlf)) => Some(crlf),
            (None, None) => None,
        }) else {
            break;
        };
        consumed = Some(next_consumed);
        cursor = next_consumed;
    }
    let consumed = consumed?;
    Some((jsonrpc_messages_from_sse_body(&bytes[..consumed]), consumed))
}

fn body_starts_like_sse(bytes: &[u8]) -> bool {
    let text = String::from_utf8_lossy(bytes);
    text.lines()
        .next()
        .map(str::trim)
        .is_some_and(|line| line.starts_with("event:") || line.starts_with("data:"))
}

fn jsonrpc_messages_from_sse_body(body: &[u8]) -> Vec<Value> {
    let text = String::from_utf8_lossy(body).replace("\r\n", "\n");
    let mut messages = Vec::new();
    for block in text.split("\n\n").filter(|block| !block.trim().is_empty()) {
        let data = block
            .lines()
            .filter_map(|line| {
                let line = line.trim_end_matches('\r');
                let (name, value) = line.split_once(':')?;
                name.trim()
                    .eq_ignore_ascii_case("data")
                    .then(|| value.trim_start())
            })
            .collect::<Vec<_>>()
            .join("\n");
        if data.trim().is_empty() || data.trim() == "[DONE]" {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(data.trim()) else {
            continue;
        };
        messages.extend(jsonrpc_messages_from_value(value));
    }
    messages
}

fn jsonrpc_messages_from_value(value: Value) -> Vec<Value> {
    match value {
        Value::Array(values) => values
            .into_iter()
            .filter(is_jsonrpc_object)
            .collect::<Vec<_>>(),
        value if is_jsonrpc_object(&value) => vec![value],
        _ => Vec::new(),
    }
}

fn is_jsonrpc_object(value: &Value) -> bool {
    value
        .get("jsonrpc")
        .and_then(Value::as_str)
        .is_some_and(|version| version == "2.0")
}

fn http_header_value<'a>(headers: &'a str, name: &str) -> Option<&'a str> {
    headers.split("\r\n").find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.eq_ignore_ascii_case(name).then(|| value.trim())
    })
}

fn http_session_id_from_server_key(server_key: &McpServerKey) -> Option<&str> {
    let McpEndpointKey::StreamableHttp(endpoint) = &server_key.endpoint else {
        return None;
    };
    endpoint.session_id.as_deref()
}

fn remote_http2_path_fallback(stream_id: Option<u32>) -> String {
    stream_id
        .map(|id| format!("h2-stream-{id}"))
        .unwrap_or_else(|| "unknown".to_string())
}

fn plaintext_http_scheme(source_boundary: &str) -> &'static str {
    if source_boundary == "Syscall" {
        "http"
    } else {
        "https"
    }
}

fn mcp_action_uses_stdio(action: &SemanticAction) -> bool {
    action
        .attributes
        .get(attrs::mcp::TRANSPORT)
        .is_some_and(|transport| transport == MCP_TRANSPORT_STDIO)
}

fn llm_tool_call_proposals(action: &SemanticAction) -> Vec<LlmToolCallProposal> {
    let Some(tool_calls_json) = action.attributes.get(attrs::llm_response::TOOL_CALLS_JSON) else {
        return Vec::new();
    };
    let Ok(Value::Array(tool_calls)) = serde_json::from_str::<Value>(tool_calls_json) else {
        return Vec::new();
    };
    tool_calls
        .iter()
        .filter_map(|tool_call| {
            let name = tool_call
                .pointer("/function/name")
                .and_then(Value::as_str)
                .or_else(|| tool_call.get("name").and_then(Value::as_str))?;
            let (server_name, tool_name) = parse_mcp_tool_call_name(name)?;
            Some(LlmToolCallProposal {
                response_action_id: action.action_id.clone(),
                tool_call_id: tool_call
                    .get("id")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty())
                    .map(ToString::to_string),
                name: name.to_string(),
                server_name,
                tool_name,
            })
        })
        .collect()
}

fn parse_mcp_tool_call_name(name: &str) -> Option<(String, String)> {
    let name = name.strip_prefix("mcp__")?;
    let (server_name, tool_name) = name.split_once("__")?;
    if server_name.is_empty() || tool_name.is_empty() {
        return None;
    }
    Some((server_name.to_string(), tool_name.to_string()))
}

fn starts_with_content_length(buffer: &[u8]) -> bool {
    buffer.len() >= "Content-Length:".len()
        && buffer[.."Content-Length:".len()].eq_ignore_ascii_case(b"Content-Length:")
}

fn leading_empty_line_len(buffer: &[u8]) -> usize {
    if buffer.starts_with(b"\r\n") {
        2
    } else if buffer.starts_with(b"\n") || buffer.starts_with(b"\r") {
        1
    } else {
        0
    }
}

fn newline_message_len(buffer: &[u8]) -> Option<usize> {
    buffer
        .iter()
        .position(|byte| *byte == b'\n')
        .map(|pos| pos + 1)
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|candidate| candidate == needle)
}

fn mcp_tool_action_id(server_key: &McpServerKey, stream_key: &str, request_id: &str) -> String {
    let stream_component = match &server_key.endpoint {
        McpEndpointKey::Stdio => sanitize_action_id_component(stream_key),
        McpEndpointKey::StreamableHttp(endpoint) => sanitize_action_id_component(&format!(
            "{}:{}:{}:{}",
            stream_key,
            endpoint.authority,
            endpoint.path,
            endpoint.session_id.as_deref().unwrap_or("")
        )),
    };
    format!(
        "trace:{}:process:{}:{}:mcp.tool_call:{}:{}",
        server_key.trace_id.get(),
        server_key.process.pid,
        server_key.process.generation,
        stream_component,
        sanitize_action_id_component(request_id)
    )
}

fn mcp_child_action_id(tool_call_action_id: &str, suffix: &str) -> String {
    format!("{tool_call_action_id}:{suffix}")
}

fn mcp_stream_pair_key(stream_key: &str) -> String {
    stream_key
        .strip_suffix("_stdin")
        .or_else(|| stream_key.strip_suffix("_stdout"))
        .or_else(|| stream_key.strip_suffix(":stdin"))
        .or_else(|| stream_key.strip_suffix(":stdout"))
        .unwrap_or(stream_key)
        .to_string()
}

fn evidence_with_role(evidence: &[SemanticEvidence], role: &str) -> Vec<SemanticEvidence> {
    evidence
        .iter()
        .map(|evidence| SemanticEvidence {
            kind: evidence.kind,
            id: evidence.id,
            role: role.to_string(),
        })
        .collect()
}

fn insert_stdio_message_attributes(
    action: &mut SemanticAction,
    value: &Value,
    direction: McpMessageDirection,
    sequence: u64,
    exchange_index: u64,
    tool_call_request_id: &str,
) {
    if let Some(message_id) = jsonrpc_id(value) {
        action
            .attributes
            .insert(attrs::mcp::MESSAGE_ID.to_string(), message_id);
    }
    if let Some(method) = jsonrpc_method(value) {
        action
            .attributes
            .insert(attrs::mcp::MESSAGE_METHOD.to_string(), method.to_string());
    }
    action.attributes.insert(
        attrs::mcp::MESSAGE_DIRECTION.to_string(),
        direction.as_str().to_string(),
    );
    action.attributes.insert(
        attrs::mcp::MESSAGE_SEQUENCE.to_string(),
        sequence.to_string(),
    );
    action.attributes.insert(
        attrs::mcp::EXCHANGE_INDEX.to_string(),
        exchange_index.to_string(),
    );
    action.attributes.insert(
        attrs::mcp::TOOL_CALL_REQUEST_ID.to_string(),
        tool_call_request_id.to_string(),
    );
}

fn sanitize_action_id_component(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    if sanitized.is_empty() {
        "unknown".to_string()
    } else {
        sanitized
    }
}
