use std::collections::BTreeMap;

use model_core::process::ProcessIdentity;
use semantic_action::{
    SemanticAction, SemanticActionCompleteness, SemanticActionKind, SemanticActionLink,
    SemanticActionLinkConfidence, SemanticActionLinkRole, SemanticActionStatus, SemanticEvidence,
    attr_keys as attrs, evidence_roles,
};
use serde_json::Value;

use super::model::{
    McpBufferedStdioMessage, McpJsonRpcId, McpJsonRpcMessage, McpOpenCall, McpRequestKey,
    McpResponseKey, McpStdioSessionKey,
};
use super::projector::{LiveMcpProjector, McpProjectionOutput};
use crate::live::actions::{append_missing_evidence, insert_parent_identity_attributes};

impl LiveMcpProjector {
    pub(super) fn observe_message(
        &mut self,
        session: &McpStdioSessionKey,
        buffered: McpBufferedStdioMessage,
    ) -> McpProjectionOutput {
        self.observe_server_metadata(session, &buffered);
        if buffered.message.is_tools_call_admission(buffered.direction) {
            return self.observe_tools_call_request(session, buffered);
        }
        if buffered.direction == super::model::McpMessageDirection::ServerToClient
            && let Some(status) = buffered.message.response_status()
        {
            return self.observe_tools_call_response(session, buffered, status);
        }
        McpProjectionOutput::default()
    }

    fn observe_tools_call_request(
        &mut self,
        session: &McpStdioSessionKey,
        buffered: McpBufferedStdioMessage,
    ) -> McpProjectionOutput {
        let request_id = buffered
            .message
            .id()
            .expect("admitted tools/call must have a string or number id");
        let tool_name = buffered
            .message
            .tools_call_name()
            .expect("admitted tools/call must have a valid tool name")
            .to_string();
        let response_key = McpResponseKey {
            session: session.clone(),
            request_id,
        };
        let invocation_sequence = self
            .correlation
            .invocation_sequences
            .entry(response_key.clone())
            .or_default();
        *invocation_sequence = invocation_sequence
            .checked_add(1)
            .expect("MCP invocation sequence overflow");
        let request_key = McpRequestKey {
            response: response_key.clone(),
            invocation_sequence: *invocation_sequence,
        };
        let mut tool_call = self.tool_call_action(
            &request_key,
            &buffered.server_process,
            &buffered.stream_key,
            &tool_name,
            &buffered.message,
        );
        tool_call.attributes.insert(
            attrs::mcp::REQUEST_ACTION_ID.to_string(),
            Self::child_action_id(&tool_call.action_id, "request"),
        );
        tool_call.attributes.insert(
            attrs::mcp::STDOUT_ACTION_ID.to_string(),
            Self::child_action_id(&tool_call.action_id, "stdout"),
        );
        let request = self.child_action(
            &tool_call,
            SemanticActionKind::McpRequest,
            "request",
            "MCP request",
            &buffered.message,
            evidence_roles::mcp::REQUEST_PAYLOAD,
            SemanticActionStatus::Success,
        );
        let mut stdout = self.child_action(
            &tool_call,
            SemanticActionKind::McpStdout,
            "stdout",
            "MCP stdout",
            &buffered.message,
            evidence_roles::mcp::STDOUT_PAYLOAD,
            SemanticActionStatus::Success,
        );
        stdout.attributes.insert(
            attrs::mcp::REQUEST_ACTION_ID.to_string(),
            request.action_id.clone(),
        );
        self.insert_message_attributes(
            &mut stdout,
            &buffered.message.value,
            buffered.direction,
            &response_key.request_id,
        );
        self.track_unattributed_mcp_tool_call(&tool_call);
        self.correlation.open_calls.insert(
            request_key.clone(),
            McpOpenCall {
                action: tool_call.clone(),
            },
        );
        self.correlation
            .open_by_response
            .entry(response_key)
            .or_default()
            .push_back(request_key);
        let contents = self
            .content
            .request(
                request.trace_id,
                &request.action_id,
                &buffered.message.value,
            )
            .into_iter()
            .collect();
        McpProjectionOutput {
            links: vec![
                self.link(
                    &tool_call,
                    &request,
                    SemanticActionLinkRole::McpToolCallRequest,
                ),
                self.link(&request, &stdout, SemanticActionLinkRole::McpRequestStdout),
            ],
            actions: vec![tool_call, request, stdout],
            contents,
            payload_segments: Vec::new(),
        }
    }

    fn observe_tools_call_response(
        &mut self,
        session: &McpStdioSessionKey,
        buffered: McpBufferedStdioMessage,
        status: SemanticActionStatus,
    ) -> McpProjectionOutput {
        let Some(request_id) = buffered.message.id() else {
            return McpProjectionOutput::default();
        };
        let response_key = McpResponseKey {
            session: session.clone(),
            request_id,
        };
        let request_key = {
            let Some(queue) = self.correlation.open_by_response.get_mut(&response_key) else {
                return McpProjectionOutput::default();
            };
            let request_key = queue.pop_front();
            if queue.is_empty() {
                self.correlation.open_by_response.remove(&response_key);
            }
            request_key
        };
        let Some(request_key) = request_key else {
            return McpProjectionOutput::default();
        };
        let Some(mut open) = self.correlation.open_calls.remove(&request_key) else {
            return McpProjectionOutput::default();
        };
        open.action.status = status;
        open.action.completeness = SemanticActionCompleteness::Complete;
        open.action.end_time = Some(buffered.message.observed_at);
        open.action.attributes.insert(
            attrs::mcp::EXECUTION_STATUS.to_string(),
            status.as_str().to_string(),
        );
        append_missing_evidence(&mut open.action.evidence, &buffered.message.evidence);
        open.action.attributes.insert(
            attrs::mcp::RESPONSE_ACTION_ID.to_string(),
            Self::child_action_id(&open.action.action_id, "response"),
        );
        open.action.attributes.insert(
            attrs::mcp::STDIN_ACTION_ID.to_string(),
            Self::child_action_id(&open.action.action_id, "stdin"),
        );
        self.track_unattributed_mcp_tool_call(&open.action);
        let response = self.child_action(
            &open.action,
            SemanticActionKind::McpResponse,
            "response",
            "MCP response",
            &buffered.message,
            evidence_roles::mcp::RESPONSE_PAYLOAD,
            status,
        );
        let mut stdin = self.child_action(
            &open.action,
            SemanticActionKind::McpStdin,
            "stdin",
            "MCP stdin",
            &buffered.message,
            evidence_roles::mcp::STDIN_PAYLOAD,
            status,
        );
        stdin.attributes.insert(
            attrs::mcp::RESPONSE_ACTION_ID.to_string(),
            response.action_id.clone(),
        );
        self.insert_message_attributes(
            &mut stdin,
            &buffered.message.value,
            buffered.direction,
            &response_key.request_id,
        );
        let contents = self
            .content
            .response(
                response.trace_id,
                &response.action_id,
                &buffered.message.value,
            )
            .into_iter()
            .collect();
        McpProjectionOutput {
            links: vec![
                self.link(
                    &open.action,
                    &response,
                    SemanticActionLinkRole::McpToolCallResponse,
                ),
                self.link(&response, &stdin, SemanticActionLinkRole::McpResponseStdin),
            ],
            actions: vec![open.action, response, stdin],
            contents,
            payload_segments: Vec::new(),
        }
    }

    fn tool_call_action(
        &mut self,
        request_key: &McpRequestKey,
        server_process: &ProcessIdentity,
        stream_key: &str,
        tool_name: &str,
        message: &McpJsonRpcMessage,
    ) -> SemanticAction {
        let session = &request_key.response.session;
        let mut attributes = BTreeMap::from([
            (attrs::mcp::TOOL_NAME.to_string(), tool_name.to_string()),
            (attrs::mcp::TOOL_ID.to_string(), tool_name.to_string()),
            (
                attrs::mcp::REQUEST_ID.to_string(),
                request_key.response.request_id.as_attribute().to_string(),
            ),
            (
                attrs::mcp::EXECUTION_STATUS.to_string(),
                SemanticActionStatus::InProgress.as_str().to_string(),
            ),
            (attrs::mcp::TRANSPORT.to_string(), "stdio".to_string()),
            (
                attrs::payload::SOURCE_BOUNDARY.to_string(),
                "stdio".to_string(),
            ),
            (
                attrs::payload::STREAM_KEY.to_string(),
                stream_key.to_string(),
            ),
        ]);
        let server_name = self
            .servers
            .get(session)
            .and_then(|state| state.name.as_ref())
            .cloned();
        if let Some(server_name) = &server_name {
            attributes.insert(attrs::mcp::SERVER_NAME.to_string(), server_name.clone());
        }
        if let Some(parent) = self.parent_process(session) {
            attributes.insert(
                attrs::mcp::CLIENT_PROCESS_ID.to_string(),
                parent.get().to_string(),
            );
            insert_parent_identity_attributes(&mut attributes, parent);
        }
        if let Some(server_name) = server_name {
            self.apply_llm_proposal_attributes(
                session.trace_id,
                &server_name,
                tool_name,
                message.observed_at,
                &mut attributes,
            );
        }
        SemanticAction {
            action_id: Self::tool_call_action_id(request_key, server_process),
            trace_id: session.trace_id,
            kind: SemanticActionKind::McpToolCall,
            title: format!("MCP tool {tool_name}"),
            start_time: message.observed_at,
            end_time: None,
            process: server_process.clone(),
            status: SemanticActionStatus::InProgress,
            completeness: SemanticActionCompleteness::Partial,
            attributes,
            evidence: Self::evidence_with_role(
                &message.evidence,
                evidence_roles::mcp::TOOL_CALL_PAYLOAD,
            ),
        }
    }

    fn child_action(
        &self,
        tool_call: &SemanticAction,
        kind: SemanticActionKind,
        suffix: &str,
        title_prefix: &str,
        message: &McpJsonRpcMessage,
        evidence_role: &str,
        status: SemanticActionStatus,
    ) -> SemanticAction {
        let mut attributes = tool_call.attributes.clone();
        attributes.insert(
            attrs::mcp::TOOL_CALL_ACTION_ID.to_string(),
            tool_call.action_id.clone(),
        );
        SemanticAction {
            action_id: Self::child_action_id(&tool_call.action_id, suffix),
            trace_id: tool_call.trace_id,
            kind,
            title: format!(
                "{title_prefix} {}",
                tool_call
                    .attributes
                    .get(attrs::mcp::TOOL_NAME)
                    .map(String::as_str)
                    .unwrap_or("tool")
            ),
            start_time: message.observed_at,
            end_time: Some(message.observed_at),
            process: tool_call.process.clone(),
            status,
            completeness: SemanticActionCompleteness::Complete,
            attributes,
            evidence: Self::evidence_with_role(&message.evidence, evidence_role),
        }
    }

    fn observe_server_metadata(
        &mut self,
        session: &McpStdioSessionKey,
        buffered: &McpBufferedStdioMessage,
    ) {
        // Replayed initialize messages provide only an optional server label;
        // this state never participates in stdio candidate admission.
        let state = self.servers.entry(session.clone()).or_default();
        match buffered.direction {
            super::model::McpMessageDirection::ClientToServer => {
                if buffered.message.method() == Some("initialize") {
                    state.pending_initialize_id = buffered.message.id();
                }
            }
            super::model::McpMessageDirection::ServerToClient => {
                let Some(id) = buffered.message.id() else {
                    return;
                };
                if state.pending_initialize_id.as_ref() != Some(&id) {
                    return;
                }
                state.pending_initialize_id = None;
                state.name = buffered
                    .message
                    .value
                    .pointer("/result/serverInfo/name")
                    .and_then(Value::as_str)
                    .filter(|name| !name.is_empty())
                    .map(ToString::to_string);
            }
        }
    }

    fn insert_message_attributes(
        &self,
        action: &mut SemanticAction,
        value: &Value,
        direction: super::model::McpMessageDirection,
        request_id: &McpJsonRpcId,
    ) {
        if let Some(message_id) = McpJsonRpcId::from_value(value) {
            action.attributes.insert(
                attrs::mcp::MESSAGE_ID.to_string(),
                message_id.as_attribute().to_string(),
            );
        }
        if let Some(method) = value.get("method").and_then(Value::as_str) {
            action
                .attributes
                .insert(attrs::mcp::MESSAGE_METHOD.to_string(), method.to_string());
        }
        action.attributes.insert(
            attrs::mcp::MESSAGE_DIRECTION.to_string(),
            direction.as_str().to_string(),
        );
        action
            .attributes
            .insert(attrs::mcp::MESSAGE_SEQUENCE.to_string(), "1".to_string());
        action
            .attributes
            .insert(attrs::mcp::EXCHANGE_INDEX.to_string(), "1".to_string());
        action.attributes.insert(
            attrs::mcp::TOOL_CALL_REQUEST_ID.to_string(),
            request_id.as_attribute().to_string(),
        );
    }

    fn link(
        &self,
        parent: &SemanticAction,
        child: &SemanticAction,
        role: SemanticActionLinkRole,
    ) -> SemanticActionLink {
        SemanticActionLink {
            trace_id: child.trace_id,
            parent_action_id: parent.action_id.clone(),
            child_action_id: child.action_id.clone(),
            role,
            confidence: SemanticActionLinkConfidence::Observed,
            valid: true,
            evidence: child.evidence.clone(),
            attributes: BTreeMap::new(),
        }
    }

    fn tool_call_action_id(
        request_key: &McpRequestKey,
        server_process: &ProcessIdentity,
    ) -> String {
        format!(
            "trace:{}:process:{}:mcp.tool_call:{}:{}:{}",
            request_key.response.session.trace_id.get(),
            server_process.get(),
            Self::sanitize_action_id_component(&request_key.response.session.action_component(),),
            Self::sanitize_action_id_component(&request_key.response.request_id.action_component(),),
            request_key.invocation_sequence,
        )
    }

    fn child_action_id(tool_call_action_id: &str, suffix: &str) -> String {
        format!("{tool_call_action_id}:{suffix}")
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
            "empty".to_string()
        } else {
            sanitized
        }
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
}
