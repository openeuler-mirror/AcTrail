use std::collections::BTreeMap;
use std::time::{Duration, UNIX_EPOCH};

use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;
use semantic_action::{
    SemanticAction, SemanticActionCompleteness, SemanticActionKind, SemanticActionLink,
    SemanticActionLinkConfidence, SemanticActionLinkRole, SemanticActionStatus, SemanticEvidence,
    SemanticEvidenceKind, attr_keys as attrs,
};

use super::{ActionDisplayProjection, DisplayChild, ROOT_PARENT_ID};

#[test]
fn orphan_http_message_falls_back_to_same_process_command() {
    let process = ProcessIdentity::new(42, 100, 100);
    let command = action(
        "command",
        SemanticActionKind::CommandInvocation,
        "agent",
        process.clone(),
        1,
    );
    let http = action(
        "connect",
        SemanticActionKind::HttpMessage,
        "CONNECT api.example.test:443",
        process,
        2,
    );

    let projection = ActionDisplayProjection::new(vec![command.clone(), http.clone()], vec![]);

    assert_eq!(
        action_ids(&projection.children(ROOT_PARENT_ID)),
        vec![command.action_id]
    );
    assert_eq!(
        action_ids(&projection.children("command")),
        vec![http.action_id]
    );
    assert!(projection.children("command")[0].link.is_none());
}

#[test]
fn semantic_link_parent_wins_over_same_process_fallback() {
    let process = ProcessIdentity::new(43, 100, 100);
    let command = action(
        "command",
        SemanticActionKind::CommandInvocation,
        "agent",
        process.clone(),
        1,
    );
    let llm_request = action(
        "llm-request",
        SemanticActionKind::LlmRequest,
        "LLM request",
        process.clone(),
        2,
    );
    let http = action(
        "post",
        SemanticActionKind::HttpMessage,
        "POST /chat/completions",
        process,
        3,
    );
    let link = link(
        "llm-request",
        "post",
        SemanticActionLinkRole::LlmRequestHttpMessage,
    );

    let projection =
        ActionDisplayProjection::new(vec![command, llm_request, http.clone()], vec![link]);

    assert_eq!(
        action_ids(&projection.children("command")),
        vec!["llm-request".to_string()]
    );
    assert_eq!(
        action_ids(&projection.children("llm-request")),
        vec![http.action_id]
    );
    assert_eq!(
        projection.children("llm-request")[0]
            .link
            .as_ref()
            .map(|link| link.role),
        Some(SemanticActionLinkRole::LlmRequestHttpMessage)
    );
}

#[test]
fn stale_llm_response_http_links_are_not_display_parents() {
    let process = ProcessIdentity::new(44, 100, 100);
    let first_response = llm_response("response-1", process.clone(), 3, 42);
    let second_response = llm_response("response-2", process.clone(), 5, 44);
    let stale_http = http_response("http-404", process.clone(), 1, 40, "404");
    let current_http = http_response("http-200", process.clone(), 2, 41, "200");
    let next_request = http_request("next-request", process, 4, 43);
    let links = vec![
        link(
            "response-1",
            "http-404",
            SemanticActionLinkRole::LlmResponseHttpMessage,
        ),
        link(
            "response-1",
            "http-200",
            SemanticActionLinkRole::LlmResponseHttpMessage,
        ),
        link(
            "response-2",
            "http-200",
            SemanticActionLinkRole::LlmResponseHttpMessage,
        ),
    ];

    let projection = ActionDisplayProjection::new(
        vec![
            first_response,
            second_response,
            stale_http,
            current_http,
            next_request,
        ],
        links,
    );

    assert_eq!(
        action_ids(&projection.children("response-1")),
        vec!["http-200".to_string()]
    );
    assert!(projection.children("response-2").is_empty());
}

#[test]
fn finalized_llm_call_uses_matching_http_error_response_time() {
    let process = ProcessIdentity::new(45, 100, 100);
    let call = finalized_llm_call("call", "request", process.clone(), 1, 600_000);
    let request = llm_request("request", process.clone(), 1);
    let http_request = http_request("http-request", process.clone(), 2, 10);
    let http_response = http_response("http-404", process, 250, 11, "404");
    let request_link = link(
        "request",
        "http-request",
        SemanticActionLinkRole::LlmRequestHttpMessage,
    );

    let projection = ActionDisplayProjection::new(
        vec![call, request, http_request, http_response],
        vec![request_link],
    );
    let call = projection
        .actions
        .iter()
        .find(|action| action.action_id == "call")
        .expect("projected actions should keep the LLM call");

    assert_eq!(call.status, SemanticActionStatus::Error);
    assert_eq!(call.completeness, SemanticActionCompleteness::Complete);
    assert_eq!(call.end_time, Some(UNIX_EPOCH + Duration::from_millis(251)));
    assert_eq!(
        call.attributes
            .get("http.response.status_code")
            .map(String::as_str),
        Some("404")
    );
    assert!(
        !call
            .attributes
            .contains_key("actrail.action.finalized_on_trace_close")
    );
}

#[test]
fn stale_llm_call_response_link_is_not_displayed() {
    let process = ProcessIdentity::new(46, 100, 100);
    let call = llm_call_with_response("call", "response-current", process.clone(), 1);
    let stale_response = llm_response("response-stale", process.clone(), 2, 50);
    let current_response = llm_response("response-current", process, 3, 51);
    let links = vec![
        link(
            "call",
            "response-stale",
            SemanticActionLinkRole::LlmCallResponse,
        ),
        link(
            "call",
            "response-current",
            SemanticActionLinkRole::LlmCallResponse,
        ),
    ];

    let projection =
        ActionDisplayProjection::new(vec![call, stale_response, current_response], links);

    assert_eq!(
        action_ids(&projection.children("call")),
        vec!["response-current".to_string()]
    );
}

#[test]
fn file_read_covered_by_bulk_read_is_not_displayed() {
    let process = ProcessIdentity::new(47, 100, 100);
    let bulk = bulk_read("bulk", process.clone(), 1, 70, 72);
    let covered = file_read("covered-read", process.clone(), 2, 70);
    let outside = file_read("outside-read", process, 3, 73);

    let projection =
        ActionDisplayProjection::new(vec![bulk, covered.clone(), outside.clone()], Vec::new());

    assert!(
        projection
            .actions
            .iter()
            .all(|action| action.action_id != covered.action_id)
    );
    assert!(
        projection
            .actions
            .iter()
            .any(|action| action.action_id == outside.action_id)
    );
}

#[test]
fn mcp_tool_call_is_command_child_and_llm_call_sibling() {
    let client = ProcessIdentity::new(48, 100, 100);
    let server = ProcessIdentity::new(49, 101, 101);
    let command = action(
        "command",
        SemanticActionKind::CommandInvocation,
        "claude",
        client.clone(),
        1,
    );
    let llm_call = llm_call_with_response("llm-call", "llm-response", client.clone(), 2);
    let llm_response = llm_response("llm-response", client, 3, 80);
    let mcp = mcp_tool_call("mcp-tool", server, 4);
    let links = vec![
        link(
            "command",
            "llm-call",
            SemanticActionLinkRole::CommandContainsLlmCall,
        ),
        link(
            "llm-call",
            "llm-response",
            SemanticActionLinkRole::LlmCallResponse,
        ),
        link(
            "command",
            "mcp-tool",
            SemanticActionLinkRole::CommandContainsMcpToolCall,
        ),
    ];

    let projection = ActionDisplayProjection::new(
        vec![
            command.clone(),
            llm_call.clone(),
            llm_response.clone(),
            mcp.clone(),
        ],
        links,
    );

    assert_eq!(
        action_ids(&projection.children("command")),
        vec![llm_call.action_id, mcp.action_id]
    );
    assert_eq!(
        projection.children("command")[1]
            .link
            .as_ref()
            .map(|link| link.role),
        Some(SemanticActionLinkRole::CommandContainsMcpToolCall)
    );
    assert_eq!(
        action_ids(&projection.children("llm-call")),
        vec![llm_response.action_id]
    );
    assert!(projection.children("mcp-tool").is_empty());
    assert!(projection.children("llm-response").is_empty());
}

#[test]
fn mcp_server_command_replaces_pid_only_wrapper_command() {
    let client = ProcessIdentity::new(50, 100, 100);
    let pid_only_server = ProcessIdentity::new(51, 101, 101);
    let observed_server = ProcessIdentity::new(51, 101, 101_001);
    let command_line = "/usr/bin/python3 mcp_stdio_recorder.py";
    let command = action(
        "command",
        SemanticActionKind::CommandInvocation,
        "claude",
        client,
        1,
    );
    let mut wrapper = action(
        "server-wrapper",
        SemanticActionKind::CommandInvocation,
        command_line,
        pid_only_server,
        2,
    );
    wrapper
        .attributes
        .insert("command.line".to_string(), command_line.to_string());
    let mut mcp_server = action(
        "server-mcp",
        SemanticActionKind::CommandInvocation,
        command_line,
        observed_server,
        3,
    );
    mcp_server
        .attributes
        .insert("command.line".to_string(), command_line.to_string());
    mcp_server
        .attributes
        .insert("invocation.kind".to_string(), "mcp".to_string());
    let links = vec![
        link(
            "command",
            "server-wrapper",
            SemanticActionLinkRole::CommandContainsCommandInvocation,
        ),
        link(
            "command",
            "server-mcp",
            SemanticActionLinkRole::CommandContainsCommandInvocation,
        ),
    ];

    let projection =
        ActionDisplayProjection::new(vec![command.clone(), wrapper.clone()], links.clone());
    assert_eq!(
        action_ids(&projection.children("command")),
        vec!["server-wrapper".to_string()]
    );

    let projection =
        ActionDisplayProjection::new(vec![command, wrapper, mcp_server.clone()], links);

    assert!(
        projection
            .actions
            .iter()
            .all(|action| action.action_id != "server-wrapper")
    );
    assert_eq!(
        action_ids(&projection.children("command")),
        vec![mcp_server.action_id]
    );
}

#[test]
fn mcp_server_command_replaces_pid_only_process_exec() {
    let client = ProcessIdentity::new(50, 100, 100);
    let pid_only_server = ProcessIdentity::new(51, 101, 101);
    let observed_server = ProcessIdentity::new(51, 101, 101_001);
    let command_line = "/usr/bin/python3 mcp_stdio_recorder.py";
    let command = action(
        "command",
        SemanticActionKind::CommandInvocation,
        "claude",
        client,
        1,
    );
    let mut process_exec = action(
        "server-exec",
        SemanticActionKind::ProcessExec,
        "/usr/bin/python3",
        pid_only_server,
        2,
    );
    process_exec
        .attributes
        .insert("command_line".to_string(), command_line.to_string());
    let mut mcp_server = action(
        "server-mcp",
        SemanticActionKind::CommandInvocation,
        command_line,
        observed_server,
        3,
    );
    mcp_server
        .attributes
        .insert("command.line".to_string(), command_line.to_string());
    mcp_server
        .attributes
        .insert("invocation.kind".to_string(), "mcp".to_string());
    let links = vec![link(
        "command",
        "server-mcp",
        SemanticActionLinkRole::CommandContainsCommandInvocation,
    )];

    let projection =
        ActionDisplayProjection::new(vec![command, process_exec, mcp_server.clone()], links);

    assert!(
        projection
            .actions
            .iter()
            .all(|action| action.action_id != "server-exec")
    );
    assert_eq!(
        action_ids(&projection.children("command")),
        vec![mcp_server.action_id]
    );
    assert_eq!(
        action_ids(&projection.children(ROOT_PARENT_ID)),
        vec!["command".to_string()]
    );
}

#[test]
fn mcp_request_response_stdio_children_nest_by_client_perspective() {
    let client = ProcessIdentity::new(50, 100, 100);
    let server = ProcessIdentity::new(51, 101, 101);
    let command = action(
        "command",
        SemanticActionKind::CommandInvocation,
        "claude",
        client,
        1,
    );
    let mcp = mcp_tool_call("mcp-tool", server.clone(), 2);
    let request = action(
        "mcp-tool:request",
        SemanticActionKind::McpRequest,
        "MCP request emit_probe",
        server.clone(),
        3,
    );
    let stdout = action(
        "mcp-tool:stdout",
        SemanticActionKind::McpStdout,
        "MCP stdout emit_probe",
        server.clone(),
        4,
    );
    let response = action(
        "mcp-tool:response",
        SemanticActionKind::McpResponse,
        "MCP response emit_probe",
        server.clone(),
        5,
    );
    let stdin = action(
        "mcp-tool:stdin",
        SemanticActionKind::McpStdin,
        "MCP stdin emit_probe",
        server,
        6,
    );
    let links = vec![
        link(
            "command",
            "mcp-tool",
            SemanticActionLinkRole::CommandContainsMcpToolCall,
        ),
        link(
            "mcp-tool",
            "mcp-tool:request",
            SemanticActionLinkRole::McpToolCallRequest,
        ),
        link(
            "mcp-tool:request",
            "mcp-tool:stdout",
            SemanticActionLinkRole::McpRequestStdout,
        ),
        link(
            "mcp-tool",
            "mcp-tool:response",
            SemanticActionLinkRole::McpToolCallResponse,
        ),
        link(
            "mcp-tool:response",
            "mcp-tool:stdin",
            SemanticActionLinkRole::McpResponseStdin,
        ),
    ];

    let projection = ActionDisplayProjection::new(
        vec![
            command.clone(),
            mcp.clone(),
            request.clone(),
            stdout.clone(),
            response.clone(),
            stdin.clone(),
        ],
        links,
    );

    assert_eq!(
        action_ids(&projection.children("command")),
        vec![mcp.action_id.clone()]
    );
    assert_eq!(
        action_ids(&projection.children("mcp-tool")),
        vec![request.action_id.clone(), response.action_id.clone()]
    );
    assert_eq!(
        action_ids(&projection.children("mcp-tool:request")),
        vec![stdout.action_id]
    );
    assert_eq!(
        action_ids(&projection.children("mcp-tool:response")),
        vec![stdin.action_id]
    );
}

#[test]
fn remote_mcp_request_response_nest_client_transport_children() {
    let client = ProcessIdentity::new(55, 105, 105);
    let command = action(
        "command",
        SemanticActionKind::CommandInvocation,
        "claude",
        client.clone(),
        1,
    );
    let mut mcp = mcp_tool_call("remote-mcp-tool", client.clone(), 2);
    mcp.attributes.insert(
        attrs::mcp::TRANSPORT.to_string(),
        "streamable_http".to_string(),
    );
    let request = action(
        "remote-mcp-tool:request",
        SemanticActionKind::McpRequest,
        "MCP request emit_remote_marker",
        client.clone(),
        3,
    );
    let client_send = action(
        "remote-mcp-tool:client_send",
        SemanticActionKind::McpClientSend,
        "MCP client send emit_remote_marker",
        client.clone(),
        4,
    );
    let response = action(
        "remote-mcp-tool:response",
        SemanticActionKind::McpResponse,
        "MCP response emit_remote_marker",
        client.clone(),
        5,
    );
    let client_receive = action(
        "remote-mcp-tool:client_receive",
        SemanticActionKind::McpClientReceive,
        "MCP client receive emit_remote_marker",
        client,
        6,
    );
    let links = vec![
        link(
            "command",
            "remote-mcp-tool",
            SemanticActionLinkRole::CommandContainsMcpToolCall,
        ),
        link(
            "remote-mcp-tool",
            "remote-mcp-tool:request",
            SemanticActionLinkRole::McpToolCallRequest,
        ),
        link(
            "remote-mcp-tool",
            "remote-mcp-tool:response",
            SemanticActionLinkRole::McpToolCallResponse,
        ),
        link(
            "remote-mcp-tool:request",
            "remote-mcp-tool:client_send",
            SemanticActionLinkRole::McpRequestClientSend,
        ),
        link(
            "remote-mcp-tool:response",
            "remote-mcp-tool:client_receive",
            SemanticActionLinkRole::McpResponseClientReceive,
        ),
    ];

    let projection = ActionDisplayProjection::new(
        vec![
            command.clone(),
            mcp.clone(),
            request.clone(),
            client_send.clone(),
            response.clone(),
            client_receive.clone(),
        ],
        links,
    );

    assert_eq!(
        action_ids(&projection.children("command")),
        vec![mcp.action_id.clone()]
    );
    assert_eq!(
        action_ids(&projection.children("remote-mcp-tool")),
        vec![request.action_id.clone(), response.action_id.clone()]
    );
    assert_eq!(
        action_ids(&projection.children("remote-mcp-tool:request")),
        vec![client_send.action_id]
    );
    assert_eq!(
        action_ids(&projection.children("remote-mcp-tool:response")),
        vec![client_receive.action_id]
    );
}

#[test]
fn mcp_additional_stdio_children_use_action_id_attributes_for_valid_links() {
    let server = ProcessIdentity::new(52, 101, 101);
    let mcp = mcp_tool_call("mcp-tool", server.clone(), 1);
    let request = action(
        "mcp-tool:request",
        SemanticActionKind::McpRequest,
        "MCP request verify_marker",
        server.clone(),
        2,
    );
    let response = action(
        "mcp-tool:response",
        SemanticActionKind::McpResponse,
        "MCP response verify_marker",
        server.clone(),
        3,
    );
    let mut ping_stdout = action(
        "mcp-tool:stdout:2",
        SemanticActionKind::McpStdout,
        "MCP stdout verify_marker",
        server.clone(),
        4,
    );
    ping_stdout.attributes.insert(
        attrs::mcp::REQUEST_ACTION_ID.to_string(),
        request.action_id.clone(),
    );
    let mut ping_stdin = action(
        "mcp-tool:stdin:2",
        SemanticActionKind::McpStdin,
        "MCP stdin verify_marker",
        server,
        5,
    );
    ping_stdin.attributes.insert(
        attrs::mcp::RESPONSE_ACTION_ID.to_string(),
        response.action_id.clone(),
    );
    let links = vec![
        link(
            "mcp-tool",
            "mcp-tool:request",
            SemanticActionLinkRole::McpToolCallRequest,
        ),
        link(
            "mcp-tool",
            "mcp-tool:response",
            SemanticActionLinkRole::McpToolCallResponse,
        ),
        link(
            "mcp-tool:request",
            "mcp-tool:stdout:2",
            SemanticActionLinkRole::McpRequestStdout,
        ),
        link(
            "mcp-tool:response",
            "mcp-tool:stdin:2",
            SemanticActionLinkRole::McpResponseStdin,
        ),
    ];

    let projection = ActionDisplayProjection::new(
        vec![
            mcp,
            request.clone(),
            response.clone(),
            ping_stdout.clone(),
            ping_stdin.clone(),
        ],
        links,
    );

    assert_eq!(
        action_ids(&projection.children("mcp-tool")),
        vec![request.action_id.clone(), response.action_id.clone()]
    );
    assert_eq!(
        action_ids(&projection.children("mcp-tool:request")),
        vec![ping_stdout.action_id]
    );
    assert_eq!(
        action_ids(&projection.children("mcp-tool:response")),
        vec![ping_stdin.action_id]
    );
}

fn action(
    id: &str,
    kind: SemanticActionKind,
    title: &str,
    process: ProcessIdentity,
    start_millis: u64,
) -> SemanticAction {
    SemanticAction {
        action_id: id.to_string(),
        trace_id: TraceId::new(1),
        kind,
        title: title.to_string(),
        start_time: UNIX_EPOCH + Duration::from_millis(start_millis),
        end_time: Some(UNIX_EPOCH + Duration::from_millis(start_millis + 1)),
        process,
        status: SemanticActionStatus::Success,
        completeness: SemanticActionCompleteness::Complete,
        confidence_millis: None,
        attributes: BTreeMap::new(),
        evidence: Vec::new(),
    }
}

fn llm_response(
    id: &str,
    process: ProcessIdentity,
    start_millis: u64,
    payload_segment_id: u64,
) -> SemanticAction {
    let mut action = action(
        id,
        SemanticActionKind::LlmResponse,
        "LLM response",
        process,
        start_millis,
    );
    action
        .attributes
        .insert("payload.stream_key".to_string(), "stream-1".to_string());
    action.evidence.push(SemanticEvidence {
        kind: SemanticEvidenceKind::PayloadSegment,
        id: payload_segment_id,
        role: "llm.response.payload".to_string(),
    });
    action
}

fn finalized_llm_call(
    id: &str,
    request_id: &str,
    process: ProcessIdentity,
    start_millis: u64,
    end_millis: u64,
) -> SemanticAction {
    let mut action = action(
        id,
        SemanticActionKind::LlmCall,
        "LLM call",
        process,
        start_millis,
    );
    action.status = SemanticActionStatus::Error;
    action.completeness = SemanticActionCompleteness::Partial;
    action.end_time = Some(UNIX_EPOCH + Duration::from_millis(end_millis));
    action.attributes.insert(
        "actrail.action.finalized_on_trace_close".to_string(),
        "true".to_string(),
    );
    action.attributes.insert(
        "llm.call.request_action_id".to_string(),
        request_id.to_string(),
    );
    action
}

fn llm_call_with_response(
    id: &str,
    response_id: &str,
    process: ProcessIdentity,
    start_millis: u64,
) -> SemanticAction {
    let mut action = action(
        id,
        SemanticActionKind::LlmCall,
        "LLM call",
        process,
        start_millis,
    );
    action.attributes.insert(
        attrs::llm_call::RESPONSE_ACTION_ID.to_string(),
        response_id.to_string(),
    );
    action
}

fn llm_request(id: &str, process: ProcessIdentity, start_millis: u64) -> SemanticAction {
    let mut action = action(
        id,
        SemanticActionKind::LlmRequest,
        "LLM request",
        process,
        start_millis,
    );
    action
        .attributes
        .insert("payload.stream_key".to_string(), "stream-1".to_string());
    action
}

fn http_response(
    id: &str,
    process: ProcessIdentity,
    start_millis: u64,
    payload_segment_id: u64,
    status_code: &str,
) -> SemanticAction {
    let mut action = http_action(
        id,
        "HTTP response",
        process,
        start_millis,
        payload_segment_id,
    );
    action
        .attributes
        .insert("direction".to_string(), "inbound".to_string());
    action
        .attributes
        .insert("status_code".to_string(), status_code.to_string());
    action
}

fn http_request(
    id: &str,
    process: ProcessIdentity,
    start_millis: u64,
    payload_segment_id: u64,
) -> SemanticAction {
    let mut action = http_action(
        id,
        "HTTP request",
        process,
        start_millis,
        payload_segment_id,
    );
    action
        .attributes
        .insert("direction".to_string(), "outbound".to_string());
    action
}

fn http_action(
    id: &str,
    title: &str,
    process: ProcessIdentity,
    start_millis: u64,
    payload_segment_id: u64,
) -> SemanticAction {
    let mut action = action(
        id,
        SemanticActionKind::HttpMessage,
        title,
        process,
        start_millis,
    );
    action
        .attributes
        .insert("stream_key".to_string(), "stream-1".to_string());
    action.attributes.insert(
        "payload_segment_id".to_string(),
        payload_segment_id.to_string(),
    );
    action.attributes.insert(
        "payload_sequence".to_string(),
        payload_segment_id.to_string(),
    );
    action
}

fn bulk_read(
    id: &str,
    process: ProcessIdentity,
    start_millis: u64,
    first_event_id: u64,
    last_event_id: u64,
) -> SemanticAction {
    let mut action = action(
        id,
        SemanticActionKind::FileBulkRead,
        "bulk read",
        process,
        start_millis,
    );
    action.attributes.insert(
        attrs::file_bulk_read::FIRST_EVENT_ID.to_string(),
        first_event_id.to_string(),
    );
    action.attributes.insert(
        attrs::file_bulk_read::LAST_EVENT_ID.to_string(),
        last_event_id.to_string(),
    );
    action
}

fn file_read(
    id: &str,
    process: ProcessIdentity,
    start_millis: u64,
    event_id: u64,
) -> SemanticAction {
    let mut action = action(
        id,
        SemanticActionKind::FileRead,
        "file read",
        process,
        start_millis,
    );
    action.evidence.push(SemanticEvidence {
        kind: SemanticEvidenceKind::Event,
        id: event_id,
        role: SemanticActionKind::FileRead.as_str().to_string(),
    });
    action
}

fn mcp_tool_call(id: &str, process: ProcessIdentity, start_millis: u64) -> SemanticAction {
    let mut action = action(
        id,
        SemanticActionKind::McpToolCall,
        "MCP tool emit_probe",
        process,
        start_millis,
    );
    action.attributes.insert(
        attrs::mcp::SERVER_NAME.to_string(),
        "actrail_probe".to_string(),
    );
    action
        .attributes
        .insert(attrs::mcp::TOOL_NAME.to_string(), "emit_probe".to_string());
    action
}

fn link(parent: &str, child: &str, role: SemanticActionLinkRole) -> SemanticActionLink {
    SemanticActionLink {
        trace_id: TraceId::new(1),
        parent_action_id: parent.to_string(),
        child_action_id: child.to_string(),
        role,
        confidence: SemanticActionLinkConfidence::Observed,
        valid: true,
        evidence: Vec::new(),
        attributes: BTreeMap::new(),
    }
}

fn action_ids(children: &[DisplayChild]) -> Vec<String> {
    children
        .iter()
        .map(|child| child.action.action_id.clone())
        .collect()
}
