use model_core::ids::EventId;
use model_core::payload::{PayloadDirection, PayloadSegmentId, PayloadSourceBoundary};
use model_core::process::{NamespaceIdentity, ProcessIdentity};
use semantic_action::{
    SemanticAction, SemanticActionKind, SemanticActionLinkRole, SemanticActionStatus,
    SemanticEvidenceKind,
};
use std::collections::BTreeSet;
use std::time::{Duration, UNIX_EPOCH};

use super::LiveSemanticActionOutput;
use super::test_support::*;

const MCP_STREAM: &str = "mcp-stdio";
const REMOTE_MCP_STREAM: &str = "remote-mcp";
const HTTP2_DATA_FRAME_TYPE: u8 = 0x0;
const HTTP2_HEADERS_FRAME_TYPE: u8 = 0x1;
const HTTP2_FRAME_HEADER_BYTES: usize = 9;
const HTTP2_MAX_FRAME_LENGTH_FIELD: usize = 0x00ff_ffff;
const HTTP2_LENGTH_BYTE_MASK: usize = 0xff;
const HTTP2_RESERVED_STREAM_ID_MASK: u32 = 0x7fff_ffff;
const TEST_MCP_PARSE_BUFFER_MAX_BYTES: u64 = 256;

#[test]
fn newline_jsonrpc_tools_call_exports_mcp_tool_call() {
    let mut runtime = runtime();
    let server = ProcessIdentity::new(2100, 6100, 7100);

    runtime.observe_payload_segment(&mcp_stdout_segment(
        server.clone(),
        0,
        line(initialize_response("actrail_probe")),
    ));

    let request_output = runtime.observe_payload_segment(&mcp_stdin_segment(
        server.clone(),
        1,
        line(tools_call_request(7, "actrail_probe.emit_probe")),
    ));
    let in_progress = mcp_action(&request_output);
    assert_eq!(in_progress.status, SemanticActionStatus::InProgress);
    assert_eq!(attr(in_progress, "mcp.server.name"), Some("actrail_probe"));
    assert_eq!(
        attr(in_progress, "mcp.tool.name"),
        Some("actrail_probe.emit_probe")
    );
    assert_eq!(
        attr(in_progress, "mcp.tool.id"),
        Some("actrail_probe.emit_probe")
    );
    assert_eq!(attr(in_progress, "mcp.request.id"), Some("7"));
    assert_eq!(attr(in_progress, "mcp.transport"), Some("stdio"));

    let response_output = runtime.observe_payload_segment(&mcp_stdout_segment(
        server,
        2,
        line(tool_success_response(7)),
    ));
    let completed = mcp_action(&response_output);
    assert_eq!(completed.status, SemanticActionStatus::Success);
    assert_eq!(attr(completed, "mcp.execution.status"), Some("success"));
    assert_eq!(completed.title, "MCP tool actrail_probe.emit_probe");
    assert!(completed.evidence.iter().any(|evidence| {
        evidence.kind == SemanticEvidenceKind::PayloadSegment
            && evidence.role == "mcp.tool_call.payload"
    }));
}

#[test]
fn split_stdio_message_uses_latest_overlapping_segment_time() {
    let mut runtime = runtime();
    let server = ProcessIdentity::new(2101, 6101, 7101);
    let request = line(tools_call_request(701, "actrail_probe.emit_probe"));
    let split_at = request.len() / 2;
    let mut first = mcp_stdin_segment(server.clone(), 10, request[..split_at].to_vec());
    first.observed_at = UNIX_EPOCH + Duration::from_secs(10);
    let mut second = mcp_stdin_segment(server, 11, request[split_at..].to_vec());
    second.observed_at = UNIX_EPOCH + Duration::from_secs(3);

    assert!(runtime.observe_payload_segment(&first).actions.is_empty());
    let output = runtime.observe_payload_segment(&second);

    let tool_call = mcp_action(&output);
    assert_eq!(tool_call.start_time, UNIX_EPOCH + Duration::from_secs(10));
}

#[test]
fn split_remote_http_message_uses_latest_overlapping_segment_time() {
    let mut runtime = runtime();
    let client = ProcessIdentity::new(2102, 6102, 7102);
    let request = remote_mcp_request_segment(
        client.clone(),
        10,
        "localhost:8123",
        "/mcp",
        None,
        &tools_call_request(702, "actrail_probe.emit_probe"),
    );
    let split_at = request.bytes.len() / 2;
    let mut first = request.clone();
    set_segment_bytes(&mut first, request.bytes[..split_at].to_vec());
    first.observed_at = UNIX_EPOCH + Duration::from_secs(10);
    let mut second = request;
    let second_bytes = second.bytes[split_at..].to_vec();
    set_segment_bytes(&mut second, second_bytes);
    second.segment_id = PayloadSegmentId::new(second.segment_id.get() + 1);
    second.operation_id += 1;
    second.sequence += 1;
    second.observed_at = UNIX_EPOCH + Duration::from_secs(3);

    assert!(runtime.observe_payload_segment(&first).actions.is_empty());
    let output = runtime.observe_payload_segment(&second);

    let tool_call = mcp_action(&output);
    assert_eq!(tool_call.start_time, UNIX_EPOCH + Duration::from_secs(10));
}

#[test]
fn server_stdin_tools_call_exports_request_and_client_stdout_children() {
    let mut runtime = runtime();
    let server = ProcessIdentity::new(2105, 6105, 7105);

    let request_output = runtime.observe_payload_segment(&mcp_stdin_segment(
        server,
        0,
        line(tools_call_request(71, "actrail_probe.emit_probe")),
    ));

    let tool_call = mcp_action(&request_output);
    let request = action_of_kind(&request_output, SemanticActionKind::McpRequest);
    let stdout = action_of_kind(&request_output, SemanticActionKind::McpStdout);

    assert_eq!(
        request.action_id,
        format!("{}:request", tool_call.action_id)
    );
    assert_eq!(stdout.action_id, format!("{}:stdout", tool_call.action_id));
    assert_eq!(request.status, SemanticActionStatus::Success);
    assert_eq!(stdout.status, SemanticActionStatus::Success);
    assert_eq!(attr(request, "mcp.request.id"), Some("71"));
    assert_eq!(attr(stdout, "mcp.request.id"), Some("71"));
    assert_eq!(attr(stdout, "mcp.message.direction"), Some("outbound"));
    assert_eq!(
        attr(request, "mcp.tool.name"),
        Some("actrail_probe.emit_probe")
    );
    assert_eq!(
        attr(stdout, "mcp.tool.name"),
        Some("actrail_probe.emit_probe")
    );
    assert!(stdout.evidence.iter().any(|evidence| {
        evidence.kind == SemanticEvidenceKind::PayloadSegment
            && evidence.role == "mcp.stdout.payload"
    }));
    assert!(request_output.links.iter().any(|link| {
        link.role == SemanticActionLinkRole::McpToolCallRequest
            && link.parent_action_id == tool_call.action_id
            && link.child_action_id == request.action_id
    }));
    assert!(request_output.links.iter().any(|link| {
        link.role == SemanticActionLinkRole::McpRequestStdout
            && link.parent_action_id == request.action_id
            && link.child_action_id == stdout.action_id
    }));
}

#[test]
fn oversized_incomplete_stdio_jsonrpc_frame_does_not_block_later_request() {
    let mut runtime = runtime_with_mcp_parse_buffer_max_bytes(TEST_MCP_PARSE_BUFFER_MAX_BYTES);
    let server = ProcessIdentity::new(2180, 6180, 7180);

    let oversized = runtime.observe_payload_segment(&mcp_stdin_segment(
        server.clone(),
        0,
        vec![b'{'; TEST_MCP_PARSE_BUFFER_MAX_BYTES as usize + 1],
    ));
    assert_no_mcp_projection(&oversized);

    let output = runtime.observe_payload_segment(&mcp_stdin_segment(
        server,
        1,
        line(tools_call_request(900, "t")),
    ));

    let tool_call = mcp_action(&output);
    assert_eq!(tool_call.status, SemanticActionStatus::InProgress);
    assert_eq!(attr(tool_call, "mcp.request.id"), Some("900"));
    assert_eq!(attr(tool_call, "mcp.tool.name"), Some("t"));
}

#[test]
fn server_stdout_tool_result_exports_response_and_client_stdin_children() {
    let mut runtime = runtime();
    let server = ProcessIdentity::new(2106, 6106, 7106);

    runtime.observe_payload_segment(&mcp_stdin_segment(
        server.clone(),
        0,
        line(tools_call_request(72, "actrail_probe.emit_probe")),
    ));
    let response_output = runtime.observe_payload_segment(&mcp_stdout_segment(
        server,
        1,
        line(tool_success_response(72)),
    ));

    let tool_call = mcp_action(&response_output);
    let response = action_of_kind(&response_output, SemanticActionKind::McpResponse);
    let stdin = action_of_kind(&response_output, SemanticActionKind::McpStdin);

    assert_eq!(
        response.action_id,
        format!("{}:response", tool_call.action_id)
    );
    assert_eq!(stdin.action_id, format!("{}:stdin", tool_call.action_id));
    assert_eq!(tool_call.status, SemanticActionStatus::Success);
    assert_eq!(response.status, SemanticActionStatus::Success);
    assert_eq!(stdin.status, SemanticActionStatus::Success);
    assert_eq!(attr(response, "mcp.request.id"), Some("72"));
    assert_eq!(attr(stdin, "mcp.request.id"), Some("72"));
    assert_eq!(attr(stdin, "mcp.message.direction"), Some("inbound"));
    assert!(stdin.evidence.iter().any(|evidence| {
        evidence.kind == SemanticEvidenceKind::PayloadSegment
            && evidence.role == "mcp.stdin.payload"
    }));
    assert!(response_output.links.iter().any(|link| {
        link.role == SemanticActionLinkRole::McpToolCallResponse
            && link.parent_action_id == tool_call.action_id
            && link.child_action_id == response.action_id
    }));
    assert!(response_output.links.iter().any(|link| {
        link.role == SemanticActionLinkRole::McpResponseStdin
            && link.parent_action_id == response.action_id
            && link.child_action_id == stdin.action_id
    }));
}

#[test]
fn open_tool_call_exports_additional_stdio_messages_under_same_request_response() {
    let mut runtime = runtime();
    let server = ProcessIdentity::new(2107, 6107, 7107);
    let stdin_stream = "mcp-stdio:stdin";
    let stdout_stream = "mcp-stdio:stdout";

    let request_output = runtime.observe_payload_segment(&mcp_stdin_segment_with_stream(
        server.clone(),
        0,
        stdin_stream,
        line(tools_call_request(3, "actrail_probe.verify_marker")),
    ));
    let tool_call = mcp_action(&request_output).clone();
    let server_ping_output = runtime.observe_payload_segment(&mcp_stdout_segment_with_stream(
        server.clone(),
        1,
        stdout_stream,
        line(ping_request("server-ping-1")),
    ));
    let ping_response_output = runtime.observe_payload_segment(&mcp_stdin_segment_with_stream(
        server.clone(),
        2,
        stdin_stream,
        line(ping_response("server-ping-1")),
    ));
    let result_output = runtime.observe_payload_segment(&mcp_stdout_segment_with_stream(
        server,
        3,
        stdout_stream,
        line(tool_success_response(3)),
    ));

    let outputs = [
        &request_output,
        &server_ping_output,
        &ping_response_output,
        &result_output,
    ];
    let actions = outputs
        .iter()
        .flat_map(|output| output.actions.iter())
        .collect::<Vec<_>>();
    let stdin = actions
        .iter()
        .copied()
        .filter(|action| action.kind == SemanticActionKind::McpStdin)
        .collect::<Vec<_>>();
    let stdout = actions
        .iter()
        .copied()
        .filter(|action| action.kind == SemanticActionKind::McpStdout)
        .collect::<Vec<_>>();

    assert_eq!(
        stdin.len(),
        2,
        "expected server ping request plus primary tool result client stdin"
    );
    assert_eq!(
        stdout.len(),
        2,
        "expected primary request plus ping response client stdout"
    );
    assert!(stdout.iter().any(|action| {
        action.action_id == format!("{}:stdout", tool_call.action_id)
            && attr(action, "mcp.message.method") == Some("tools/call")
            && attr(action, "mcp.message.sequence") == Some("1")
            && attr(action, "mcp.exchange.index") == Some("1")
    }));
    let ping_stdin = stdin
        .iter()
        .copied()
        .find(|action| attr(action, "mcp.message.method") == Some("ping"))
        .expect("server ping request should be projected as client stdin");
    assert_eq!(
        ping_stdin.action_id,
        format!("{}:stdin:2", tool_call.action_id)
    );
    assert_eq!(attr(ping_stdin, "mcp.message.id"), Some("server-ping-1"));
    assert_eq!(attr(ping_stdin, "mcp.message.direction"), Some("inbound"));
    assert_eq!(attr(ping_stdin, "mcp.message.sequence"), Some("2"));
    assert_eq!(attr(ping_stdin, "mcp.exchange.index"), Some("2"));
    assert_eq!(attr(ping_stdin, "mcp.tool_call.request_id"), Some("3"));

    let ping_stdout = stdout
        .iter()
        .copied()
        .find(|action| attr(action, "mcp.message.id") == Some("server-ping-1"))
        .expect("client ping response should be projected as client stdout");
    assert_eq!(
        ping_stdout.action_id,
        format!("{}:stdout:2", tool_call.action_id)
    );
    assert_eq!(attr(ping_stdout, "mcp.message.direction"), Some("outbound"));
    assert_eq!(attr(ping_stdout, "mcp.message.sequence"), Some("2"));
    assert_eq!(attr(ping_stdout, "mcp.exchange.index"), Some("2"));
    assert_eq!(attr(ping_stdout, "mcp.tool_call.request_id"), Some("3"));

    assert!(stdin.iter().any(|action| {
        action.action_id == format!("{}:stdin", tool_call.action_id)
            && attr(action, "mcp.message.id") == Some("3")
            && attr(action, "mcp.message.sequence") == Some("1")
            && attr(action, "mcp.exchange.index") == Some("1")
    }));

    let links = outputs
        .iter()
        .flat_map(|output| output.links.iter())
        .collect::<Vec<_>>();
    assert!(links.iter().any(|link| {
        link.role == SemanticActionLinkRole::McpRequestStdout
            && link.parent_action_id == format!("{}:request", tool_call.action_id)
            && link.child_action_id == ping_stdout.action_id
    }));
    assert!(links.iter().any(|link| {
        link.role == SemanticActionLinkRole::McpResponseStdin
            && link.parent_action_id == format!("{}:response", tool_call.action_id)
            && link.child_action_id == ping_stdin.action_id
    }));
}

#[test]
fn content_length_framed_jsonrpc_across_segments_exports_tool_call() {
    let mut runtime = runtime();
    let server = ProcessIdentity::new(2101, 6101, 7101);
    let request = framed(tools_call_request(11, "actrail_probe.emit_probe"));
    let split_at = request.len() / 2;

    let first = runtime.observe_payload_segment(&mcp_stdin_segment(
        server.clone(),
        0,
        request[..split_at].to_vec(),
    ));
    assert!(
        first
            .actions
            .iter()
            .all(|action| action.kind.as_str() != "mcp.tool_call"),
        "partial frame must not emit an MCP tool action"
    );

    let second = runtime.observe_payload_segment(&mcp_stdin_segment(
        server.clone(),
        1,
        request[split_at..].to_vec(),
    ));
    let in_progress = mcp_action(&second);
    assert_eq!(in_progress.status, SemanticActionStatus::InProgress);
    assert_eq!(
        attr(in_progress, "mcp.tool.name"),
        Some("actrail_probe.emit_probe")
    );

    let response = runtime.observe_payload_segment(&mcp_stdout_segment(
        server,
        2,
        framed(tool_success_response(11)),
    ));
    let completed = mcp_action(&response);
    assert_eq!(completed.status, SemanticActionStatus::Success);
    assert_eq!(attr(completed, "mcp.request.id"), Some("11"));
}

#[test]
fn error_response_marks_mcp_tool_call_error() {
    let mut runtime = runtime();
    let server = ProcessIdentity::new(2102, 6102, 7102);

    runtime.observe_payload_segment(&mcp_stdin_segment(
        server.clone(),
        0,
        line(tools_call_request(12, "actrail_probe.emit_probe")),
    ));
    let response = runtime.observe_payload_segment(&mcp_stdout_segment(
        server,
        1,
        line(tool_error_response(12)),
    ));

    let completed = mcp_action(&response);
    assert_eq!(completed.status, SemanticActionStatus::Error);
    assert_eq!(attr(completed, "mcp.execution.status"), Some("error"));
}

#[test]
fn stdout_tool_result_infers_unique_tool_when_stdin_is_unseen() {
    let mut runtime = runtime();
    let server = ProcessIdentity::new(2103, 6103, 7103);

    runtime.observe_payload_segment(&mcp_stdout_segment(
        server.clone(),
        0,
        line(initialize_response("actrail_probe")),
    ));
    runtime.observe_payload_segment(&mcp_stdout_segment(
        server.clone(),
        1,
        line(tools_list_response(2, &["actrail_probe.emit_probe"])),
    ));

    let response = runtime.observe_payload_segment(&mcp_stdout_segment(
        server,
        2,
        line(tool_success_response(99)),
    ));

    let inferred = mcp_action(&response);
    assert_eq!(inferred.status, SemanticActionStatus::Success);
    assert_eq!(
        attr(inferred, "mcp.tool.name"),
        Some("actrail_probe.emit_probe")
    );
    assert_eq!(
        attr(inferred, "mcp.evidence.mode"),
        Some("response_inferred_tool")
    );
    assert_eq!(attr(inferred, "mcp.request.id"), Some("99"));
    assert!(
        response
            .actions
            .iter()
            .all(|action| action.kind != SemanticActionKind::McpRequest
                && action.kind != SemanticActionKind::McpStdout),
        "server-stdout-only inference must not invent unobserved request/client-stdout actions"
    );
    assert!(response.actions.iter().any(|action| {
        action.kind == SemanticActionKind::McpResponse
            && action.action_id == format!("{}:response", inferred.action_id)
    }));
    assert!(response.actions.iter().any(|action| {
        action.kind == SemanticActionKind::McpStdin
            && action.action_id == format!("{}:stdin", inferred.action_id)
    }));
}

#[test]
fn stdout_tool_result_does_not_infer_when_server_has_multiple_tools() {
    let mut runtime = runtime();
    let server = ProcessIdentity::new(2104, 6104, 7104);

    runtime.observe_payload_segment(&mcp_stdout_segment(
        server.clone(),
        0,
        line(tools_list_response(
            2,
            &["actrail_probe.emit_probe", "actrail_probe.read_probe"],
        )),
    ));
    let response = runtime.observe_payload_segment(&mcp_stdout_segment(
        server,
        1,
        line(tool_success_response(100)),
    ));

    assert!(
        response
            .actions
            .iter()
            .all(|action| action.kind.as_str() != "mcp.tool_call"),
        "multi-tool stdout-only responses must not guess a tool"
    );
}

#[test]
fn mcp_tool_call_attrs_include_server_parent_client_pid() {
    let mut runtime = runtime();
    let client = ProcessIdentity::new(4100, 8100, 9100);
    let server = ProcessIdentity::new(4101, 8101, 9101);

    runtime.observe_event(&exec_event(
        EventId::new(610),
        server.clone(),
        Some(client.clone()),
        "node /tmp/actrail-mcp-server.js",
    ));
    runtime.observe_payload_segment(&mcp_stdin_segment(
        server.clone(),
        0,
        line(tools_call_request(44, "actrail_probe.emit_probe")),
    ));
    let response = runtime.observe_payload_segment(&mcp_stdout_segment(
        server,
        1,
        line(tool_success_response(44)),
    ));

    let completed = mcp_action(&response);
    assert_eq!(
        attr(completed, "process.parent.pid"),
        Some(client.pid.to_string().as_str())
    );
    assert_eq!(
        attr(completed, "mcp.client.pid"),
        Some(client.pid.to_string().as_str())
    );
}

#[test]
fn mcp_tool_call_adds_parent_pid_when_server_exec_arrives_after_stdout_result() {
    let mut runtime = runtime();
    let client = ProcessIdentity::new(4200, 8200, 9200);
    let server = ProcessIdentity::new(4201, 8201, 9201);
    let server_exec =
        ProcessIdentity::new(4201, 8201, 9201).with_namespace(NamespaceIdentity::new("pid:[mcp]"));

    runtime.observe_payload_segment(&mcp_stdout_segment(
        server.clone(),
        0,
        line(initialize_response("actrail_probe")),
    ));
    runtime.observe_payload_segment(&mcp_stdout_segment(
        server.clone(),
        1,
        line(tools_list_response(2, &["emit_probe"])),
    ));
    let response = runtime.observe_payload_segment(&mcp_stdout_segment(
        server.clone(),
        2,
        line(tool_success_response(99)),
    ));
    let inferred = mcp_action(&response);
    assert_eq!(attr(inferred, "process.parent.pid"), None);
    assert_eq!(attr(inferred, "mcp.client.pid"), None);

    let update = runtime.observe_event(&exec_event(
        EventId::new(611),
        server_exec,
        Some(client.clone()),
        "python3 /tmp/actrail-mcp-server.py",
    ));

    let updated = mcp_action(&update);
    assert_eq!(updated.action_id, inferred.action_id);
    assert_eq!(
        attr(updated, "process.parent.pid"),
        Some(client.pid.to_string().as_str())
    );
    assert_eq!(
        attr(updated, "mcp.client.pid"),
        Some(client.pid.to_string().as_str())
    );
}

#[test]
fn mcp_tool_call_links_under_client_command_and_keeps_server_command_lineage() {
    let mut runtime = runtime();
    let client = ProcessIdentity::new(4300, 8300, 9300);
    let server = ProcessIdentity::new(4301, 8301, 9301);

    let client_output = runtime.observe_event(&exec_event(
        EventId::new(620),
        client.clone(),
        None,
        "/root/.claude/local/claude",
    ));
    let client_command = command_action(&client_output);

    let server_output = runtime.observe_event(&exec_event(
        EventId::new(621),
        server.clone(),
        Some(client.clone()),
        "node /tmp/actrail-mcp-server.js",
    ));
    let server_command = command_action(&server_output);
    assert!(server_output.links.iter().any(|link| {
        link.role == SemanticActionLinkRole::CommandContainsCommandInvocation
            && link.parent_action_id == client_command.action_id
            && link.child_action_id == server_command.action_id
    }));

    runtime.observe_payload_segment(&mcp_stdout_segment(
        server.clone(),
        0,
        line(initialize_response("actrail_probe")),
    ));
    let request_output = runtime.observe_payload_segment(&mcp_stdin_segment(
        server,
        1,
        line(tools_call_request(45, "emit_probe")),
    ));
    let mcp = mcp_action(&request_output);
    let link = request_output
        .links
        .iter()
        .find(|link| {
            link.role == SemanticActionLinkRole::CommandContainsMcpToolCall
                && link.child_action_id == mcp.action_id
        })
        .expect("MCP execution should link under the client command");

    assert_eq!(link.parent_action_id, client_command.action_id);
    assert!(link.evidence.iter().any(|evidence| {
        evidence.kind == SemanticEvidenceKind::PayloadSegment
            && evidence.role == "mcp.tool_call.payload"
    }));
}

#[test]
fn mcp_tool_call_links_to_client_command_when_parent_identity_arrives_late() {
    let mut runtime = runtime();
    let client = ProcessIdentity::new(4400, 8400, 9400);
    let server = ProcessIdentity::new(4401, 8401, 9401);

    let client_output = runtime.observe_event(&exec_event(
        EventId::new(630),
        client.clone(),
        None,
        "/root/.claude/local/claude",
    ));
    let client_command = command_action(&client_output);

    runtime.observe_payload_segment(&mcp_stdout_segment(
        server.clone(),
        0,
        line(initialize_response("actrail_probe")),
    ));
    runtime.observe_payload_segment(&mcp_stdout_segment(
        server.clone(),
        1,
        line(tools_list_response(2, &["emit_probe"])),
    ));
    let response = runtime.observe_payload_segment(&mcp_stdout_segment(
        server.clone(),
        2,
        line(tool_success_response(46)),
    ));
    let inferred = mcp_action(&response);
    assert!(response.links.iter().all(|link| {
        link.role != SemanticActionLinkRole::CommandContainsMcpToolCall
            || link.child_action_id != inferred.action_id
    }));

    let update = runtime.observe_event(&exec_event(
        EventId::new(631),
        server,
        Some(client),
        "python3 /tmp/actrail-mcp-server.py",
    ));
    let updated = mcp_action(&update);
    let link = update
        .links
        .iter()
        .find(|link| {
            link.role == SemanticActionLinkRole::CommandContainsMcpToolCall
                && link.child_action_id == updated.action_id
        })
        .expect("late parent identity should link the MCP action under the client command");

    assert_eq!(updated.action_id, inferred.action_id);
    assert_eq!(link.parent_action_id, client_command.action_id);
}

#[test]
fn mcp_tool_call_with_different_client_pid_does_not_link_to_unrelated_command() {
    let mut runtime = runtime();
    let client = ProcessIdentity::new(4500, 8500, 9500);
    let unrelated = ProcessIdentity::new(4502, 8502, 9502);
    let server = ProcessIdentity::new(4501, 8501, 9501);

    runtime.observe_event(&exec_event(
        EventId::new(640),
        client,
        None,
        "/root/.claude/local/claude",
    ));
    runtime.observe_event(&exec_event(
        EventId::new(641),
        server.clone(),
        Some(unrelated),
        "node /tmp/actrail-mcp-server.js",
    ));
    let request_output = runtime.observe_payload_segment(&mcp_stdin_segment(
        server,
        0,
        line(tools_call_request(47, "emit_probe")),
    ));
    let mcp = mcp_action(&request_output);

    assert!(request_output.links.iter().all(|link| {
        link.role != SemanticActionLinkRole::CommandContainsMcpToolCall
            || link.child_action_id != mcp.action_id
    }));
}

#[test]
fn mcp_tool_call_carries_matching_llm_tool_call_proposal() {
    let mut runtime = runtime();
    let client = ProcessIdentity::new(4600, 8600, 9600);
    let server = ProcessIdentity::new(4601, 8601, 9601);

    runtime.observe_event(&exec_event(
        EventId::new(650),
        client.clone(),
        None,
        "/root/.claude/local/claude",
    ));
    runtime.observe_event(&exec_event(
        EventId::new(651),
        server.clone(),
        Some(client.clone()),
        "node /tmp/actrail-mcp-server.js",
    ));
    let llm_output = runtime.observe_payload_segment(&llm_response_with_tool_call(
        client,
        "mcp__actrail_probe__emit_probe",
    ));
    let llm_response = llm_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmResponse)
        .expect("LLM response should project from tool-call SSE payload");

    runtime.observe_payload_segment(&mcp_stdout_segment(
        server.clone(),
        0,
        line(initialize_response("actrail_probe")),
    ));
    let request_output = runtime.observe_payload_segment(&mcp_stdin_segment(
        server,
        1,
        line(tools_call_request(48, "emit_probe")),
    ));
    let mcp = mcp_action(&request_output);

    assert_eq!(
        attr(mcp, "llm.response.action_id"),
        Some(llm_response.action_id.as_str())
    );
    assert_eq!(attr(mcp, "llm.tool_call.id"), Some("call_mcp_1"));
    assert_eq!(
        attr(mcp, "llm.tool_call.name"),
        Some("mcp__actrail_probe__emit_probe")
    );
}

#[test]
fn mcp_tool_call_marks_existing_server_command_invocation_as_mcp() {
    let mut runtime = runtime();
    let client = ProcessIdentity::new(4700, 8700, 9700);
    let server = ProcessIdentity::new(4701, 8701, 9701);

    runtime.observe_event(&exec_event(
        EventId::new(660),
        server.clone(),
        Some(client),
        "node /tmp/actrail-mcp-server.js",
    ));
    let request_output = runtime.observe_payload_segment(&mcp_stdin_segment(
        server.clone(),
        0,
        line(tools_call_request(49, "emit_probe")),
    ));

    let command = command_action(&request_output);
    assert_eq!(command.process, server);
    assert_eq!(attr(command, "invocation.kind"), Some("mcp"));
    assert!(command.evidence.iter().any(|evidence| {
        evidence.kind == SemanticEvidenceKind::PayloadSegment
            && evidence.role == "mcp.tool_call.payload"
    }));
}

#[test]
fn mcp_tool_call_before_exec_marks_later_server_command_invocation_as_mcp() {
    let mut runtime = runtime();
    let client = ProcessIdentity::new(4800, 8800, 9800);
    let server = ProcessIdentity::new(4801, 8801, 9801);

    runtime.observe_payload_segment(&mcp_stdin_segment(
        server.clone(),
        0,
        line(tools_call_request(50, "emit_probe")),
    ));
    let exec_output = runtime.observe_event(&exec_event(
        EventId::new(670),
        server.clone(),
        Some(client),
        "python3 /tmp/actrail-mcp-server.py",
    ));

    let command = command_action(&exec_output);
    assert_eq!(command.process, server);
    assert_eq!(attr(command, "invocation.kind"), Some("mcp"));
    assert!(command.evidence.iter().any(|evidence| {
        evidence.kind == SemanticEvidenceKind::PayloadSegment
            && evidence.role == "mcp.tool_call.payload"
    }));
}

#[test]
fn repeated_mcp_tool_calls_do_not_emit_redundant_command_updates() {
    let mut runtime = runtime();
    let client = ProcessIdentity::new(4900, 8900, 9900);
    let server = ProcessIdentity::new(4901, 8901, 9901);

    runtime.observe_event(&exec_event(
        EventId::new(680),
        server.clone(),
        Some(client),
        "node /tmp/actrail-mcp-server.js",
    ));
    let first = runtime.observe_payload_segment(&mcp_stdin_segment(
        server.clone(),
        0,
        line(tools_call_request(51, "emit_probe")),
    ));
    assert_eq!(attr(command_action(&first), "invocation.kind"), Some("mcp"));

    let second = runtime.observe_payload_segment(&mcp_stdin_segment(
        server,
        1,
        line(tools_call_request(52, "read_probe")),
    ));

    assert!(
        second
            .actions
            .iter()
            .all(|action| action.kind != SemanticActionKind::CommandInvocation),
        "second MCP tool call should not re-emit an unchanged server command"
    );
}

#[test]
fn remote_http_tools_call_exports_tool_call_request_and_client_send() {
    let mut runtime = runtime();
    let client = ProcessIdentity::new(5100, 9100, 10100);

    let output = runtime.observe_payload_segment(&remote_mcp_request_segment(
        client.clone(),
        0,
        "remote.example.test",
        "/mcp",
        Some("sess-123"),
        &tools_call_request(81, "actrail_remote_probe.emit_remote_marker"),
    ));

    let tool_call = mcp_action(&output);
    let request = action_of_kind(&output, SemanticActionKind::McpRequest);
    let client_send = action_of_kind(&output, SemanticActionKind::McpClientSend);

    assert_eq!(tool_call.process, client);
    assert_eq!(
        request.action_id,
        format!("{}:request", tool_call.action_id)
    );
    assert_eq!(
        client_send.action_id,
        format!("{}:client_send", tool_call.action_id)
    );
    assert_eq!(tool_call.status, SemanticActionStatus::InProgress);
    assert_eq!(request.status, SemanticActionStatus::Success);
    assert_eq!(client_send.status, SemanticActionStatus::Success);
    assert_eq!(attr(tool_call, "mcp.transport"), Some("streamable_http"));
    assert_eq!(attr(tool_call, "mcp.client.pid"), Some("5100"));
    assert_eq!(
        attr(tool_call, "mcp.tool.name"),
        Some("actrail_remote_probe.emit_remote_marker")
    );
    assert_eq!(attr(tool_call, "mcp.request.id"), Some("81"));
    assert_eq!(attr(tool_call, "mcp.session.id"), Some("sess-123"));
    assert_eq!(attr(tool_call, "http.request.method"), Some("POST"));
    assert_eq!(
        attr(tool_call, "server.address"),
        Some("remote.example.test")
    );
    assert_eq!(attr(tool_call, "url.path"), Some("/mcp"));
    assert_eq!(
        attr(tool_call, "payload.stream_key"),
        Some(REMOTE_MCP_STREAM)
    );
    assert!(request.evidence.iter().any(|evidence| {
        evidence.kind == SemanticEvidenceKind::PayloadSegment
            && evidence.role == "mcp.request.payload"
    }));
    assert_eq!(
        attr(request, "mcp.client_send.action_id"),
        Some(client_send.action_id.as_str())
    );
    assert_eq!(
        attr(client_send, "mcp.request.action_id"),
        Some(request.action_id.as_str())
    );
    assert_eq!(attr(client_send, "mcp.message.direction"), Some("outbound"));
    assert!(client_send.evidence.iter().any(|evidence| {
        evidence.kind == SemanticEvidenceKind::PayloadSegment
            && evidence.role == "mcp.client_send.payload"
    }));
    assert!(output.actions.iter().all(|action| {
        action.kind != SemanticActionKind::McpStdin && action.kind != SemanticActionKind::McpStdout
    }));
}

#[test]
fn oversized_non_http_remote_candidate_does_not_block_later_request() {
    let mut runtime = runtime_with_mcp_parse_buffer_max_bytes(TEST_MCP_PARSE_BUFFER_MAX_BYTES);
    let client = ProcessIdentity::new(5180, 9180, 10180);
    let mut oversized_segment = outbound_http1_payload_segment_with_bytes(
        client.clone(),
        PayloadSegmentId::new(2200),
        2300,
        0,
        vec![b'x'; TEST_MCP_PARSE_BUFFER_MAX_BYTES as usize + 1],
    );
    oversized_segment.stream_key = model_core::payload::PayloadStreamKey::new(REMOTE_MCP_STREAM);

    let oversized = runtime.observe_payload_segment(&oversized_segment);
    assert_no_mcp_projection(&oversized);

    let output = runtime.observe_payload_segment(&remote_mcp_request_segment(
        client,
        1,
        "h",
        "/m",
        None,
        &tools_call_request(901, "t"),
    ));

    let tool_call = mcp_action(&output);
    assert_eq!(tool_call.status, SemanticActionStatus::InProgress);
    assert_eq!(attr(tool_call, "mcp.request.id"), Some("901"));
    assert_eq!(attr(tool_call, "mcp.tool.name"), Some("t"));
}

#[test]
fn remote_http2_interleaved_tools_calls_export_separate_stream_actions() {
    let mut runtime = runtime();
    let client = ProcessIdentity::new(5109, 9109, 10109);
    let bytes = remote_mcp_http2_requests([
        (
            1,
            tools_call_request(91, "actrail_remote_probe.emit_remote_marker"),
        ),
        (
            3,
            tools_call_request(92, "actrail_remote_probe.read_remote_marker"),
        ),
    ]);

    let output =
        runtime.observe_payload_segment(&remote_mcp_http2_request_raw_segment(client, 0, bytes));

    let tool_calls = output
        .actions
        .iter()
        .filter(|action| action.kind == SemanticActionKind::McpToolCall)
        .collect::<Vec<_>>();
    let client_sends = output
        .actions
        .iter()
        .filter(|action| action.kind == SemanticActionKind::McpClientSend)
        .collect::<Vec<_>>();
    assert_eq!(tool_calls.len(), 2, "expected one tool call per h2 stream");
    assert_eq!(
        client_sends.len(),
        2,
        "expected one client_send child per h2 stream"
    );

    let tool_request_streams = tool_calls
        .iter()
        .map(|action| {
            (
                attr(action, "mcp.request.id")
                    .unwrap_or_default()
                    .to_string(),
                attr(action, "http.request.stream_id")
                    .unwrap_or_default()
                    .to_string(),
            )
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        tool_request_streams,
        BTreeSet::from([
            ("91".to_string(), "1".to_string()),
            ("92".to_string(), "3".to_string()),
        ])
    );

    let client_send_streams = client_sends
        .iter()
        .map(|action| {
            (
                attr(action, "mcp.request.id")
                    .unwrap_or_default()
                    .to_string(),
                attr(action, "http.request.stream_id")
                    .unwrap_or_default()
                    .to_string(),
            )
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        client_send_streams,
        BTreeSet::from([
            ("91".to_string(), "1".to_string()),
            ("92".to_string(), "3".to_string()),
        ])
    );
}

#[test]
fn remote_http_syscall_request_uses_http_scheme() {
    let mut runtime = runtime();
    let client = ProcessIdentity::new(5106, 9106, 10106);
    let mut segment = remote_mcp_request_segment(
        client,
        0,
        "127.0.0.1:42723",
        "/mcp",
        None,
        &tools_call_request(87, "actrail_remote_probe.emit_remote_marker"),
    );
    segment.source_boundary = PayloadSourceBoundary::Syscall;

    let output = runtime.observe_payload_segment(&segment);
    let tool_call = mcp_action(&output);

    assert_eq!(attr(tool_call, "url.scheme"), Some("http"));
    assert_eq!(attr(tool_call, "payload.source_boundary"), Some("Syscall"));
}

#[test]
fn remote_http_json_response_completes_matching_tool_call() {
    let mut runtime = runtime();
    let client = ProcessIdentity::new(5101, 9101, 10101);

    runtime.observe_payload_segment(&remote_mcp_request_segment(
        client.clone(),
        0,
        "remote.example.test",
        "/mcp",
        None,
        &tools_call_request(82, "actrail_remote_probe.emit_remote_marker"),
    ));
    let output = runtime.observe_payload_segment(&remote_mcp_response_segment(
        client,
        1,
        Some("application/json"),
        None,
        &tool_success_response(82),
    ));

    let completed = mcp_action(&output);
    let response = action_of_kind(&output, SemanticActionKind::McpResponse);
    let client_receive = action_of_kind(&output, SemanticActionKind::McpClientReceive);

    assert_eq!(completed.status, SemanticActionStatus::Success);
    assert_eq!(completed.completeness.as_str(), "complete");
    assert_eq!(attr(completed, "mcp.execution.status"), Some("success"));
    assert_eq!(attr(completed, "mcp.transport"), Some("streamable_http"));
    assert_eq!(
        attr(completed, "mcp.tool.name"),
        Some("actrail_remote_probe.emit_remote_marker")
    );
    assert_eq!(
        response.action_id,
        format!("{}:response", completed.action_id)
    );
    assert_eq!(
        client_receive.action_id,
        format!("{}:client_receive", completed.action_id)
    );
    assert_eq!(response.status, SemanticActionStatus::Success);
    assert_eq!(client_receive.status, SemanticActionStatus::Success);
    assert!(response.evidence.iter().any(|evidence| {
        evidence.kind == SemanticEvidenceKind::PayloadSegment
            && evidence.role == "mcp.response.payload"
    }));
    assert_eq!(
        attr(response, "mcp.client_receive.action_id"),
        Some(client_receive.action_id.as_str())
    );
    assert_eq!(
        attr(client_receive, "mcp.response.action_id"),
        Some(response.action_id.as_str())
    );
    assert_eq!(
        attr(client_receive, "mcp.message.direction"),
        Some("inbound")
    );
    assert!(client_receive.evidence.iter().any(|evidence| {
        evidence.kind == SemanticEvidenceKind::PayloadSegment
            && evidence.role == "mcp.client_receive.payload"
    }));
    assert!(output.actions.iter().all(|action| {
        action.kind != SemanticActionKind::McpStdin && action.kind != SemanticActionKind::McpStdout
    }));
}

#[test]
fn remote_http_error_responses_mark_tool_call_error() {
    let mut runtime = runtime();
    let client = ProcessIdentity::new(5102, 9102, 10102);

    runtime.observe_payload_segment(&remote_mcp_request_segment(
        client.clone(),
        0,
        "remote.example.test",
        "/mcp",
        None,
        &tools_call_request(83, "actrail_remote_probe.emit_remote_marker"),
    ));
    let jsonrpc_error = runtime.observe_payload_segment(&remote_mcp_response_segment(
        client.clone(),
        1,
        Some("application/json"),
        None,
        &tool_error_response(83),
    ));
    assert_eq!(
        mcp_action(&jsonrpc_error).status,
        SemanticActionStatus::Error
    );
    assert_eq!(
        attr(mcp_action(&jsonrpc_error), "mcp.execution.status"),
        Some("error")
    );

    runtime.observe_payload_segment(&remote_mcp_request_segment(
        client.clone(),
        2,
        "remote.example.test",
        "/mcp",
        None,
        &tools_call_request(84, "actrail_remote_probe.emit_remote_marker"),
    ));
    let tool_error = runtime.observe_payload_segment(&remote_mcp_response_segment(
        client,
        3,
        Some("application/json"),
        None,
        &tool_result_is_error_response(84),
    ));
    assert_eq!(mcp_action(&tool_error).status, SemanticActionStatus::Error);
    assert_eq!(
        attr(mcp_action(&tool_error), "mcp.execution.status"),
        Some("error")
    );
}

#[test]
fn remote_http_sse_response_completes_matching_tool_call() {
    let mut runtime = runtime();
    let client = ProcessIdentity::new(5103, 9103, 10103);

    runtime.observe_payload_segment(&remote_mcp_request_segment(
        client.clone(),
        0,
        "remote.example.test",
        "/mcp",
        None,
        &tools_call_request(85, "actrail_remote_probe.emit_remote_marker"),
    ));
    let body = format!("event: message\ndata: {}\n\n", tool_success_response(85));
    let output = runtime.observe_payload_segment(&remote_mcp_response_segment(
        client,
        1,
        Some("text/event-stream"),
        None,
        &body,
    ));

    let completed = mcp_action(&output);
    let response = action_of_kind(&output, SemanticActionKind::McpResponse);
    let client_receive = action_of_kind(&output, SemanticActionKind::McpClientReceive);
    assert_eq!(completed.status, SemanticActionStatus::Success);
    assert_eq!(response.status, SemanticActionStatus::Success);
    assert_eq!(client_receive.status, SemanticActionStatus::Success);
    assert_eq!(attr(response, "mcp.transport"), Some("streamable_http"));
    assert!(client_receive.evidence.iter().any(|evidence| {
        evidence.kind == SemanticEvidenceKind::PayloadSegment
            && evidence.role == "mcp.client_receive.payload"
    }));
}

#[test]
fn remote_http_chunked_sse_response_completes_before_terminal_chunk() {
    let mut runtime = runtime();
    let client = ProcessIdentity::new(5110, 9110, 10110);

    runtime.observe_payload_segment(&remote_mcp_request_segment(
        client.clone(),
        0,
        "remote.example.test",
        "/mcp",
        None,
        &tools_call_request(93, "actrail_remote_probe.emit_remote_marker"),
    ));

    let body = format!("event: message\ndata: {}\n\n", tool_success_response(93));
    let mut bytes =
        b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n"
            .to_vec();
    bytes.extend(http_chunk_prefix(&body));
    bytes.extend(body.as_bytes());
    bytes.extend(b"\r\n");
    let output =
        runtime.observe_payload_segment(&remote_mcp_response_raw_segment(client, 1, bytes));

    let completed = mcp_action(&output);
    let response = action_of_kind(&output, SemanticActionKind::McpResponse);
    let client_receive = action_of_kind(&output, SemanticActionKind::McpClientReceive);
    assert_eq!(completed.status, SemanticActionStatus::Success);
    assert_eq!(response.status, SemanticActionStatus::Success);
    assert_eq!(client_receive.status, SemanticActionStatus::Success);
    assert_eq!(attr(completed, "mcp.request.id"), Some("93"));
}

#[test]
fn remote_http2_sse_response_completes_matching_tool_call() {
    let mut runtime = runtime();
    let client = ProcessIdentity::new(5111, 9111, 10111);

    runtime.observe_payload_segment(&remote_mcp_http2_request_raw_segment(
        client.clone(),
        0,
        remote_mcp_http2_requests([(
            5,
            tools_call_request(94, "actrail_remote_probe.emit_remote_marker"),
        )]),
    ));

    let body = format!("event: message\ndata: {}\n\n", tool_success_response(94));
    let split_at = "event: message\n".len();
    let partial = runtime.observe_payload_segment(&remote_mcp_http2_response_raw_segment(
        client.clone(),
        1,
        remote_mcp_http2_response(5, &body.as_bytes()[..split_at]),
    ));
    assert!(
        partial.actions.iter().all(|action| {
            action.kind != SemanticActionKind::McpToolCall
                && action.kind != SemanticActionKind::McpResponse
                && action.kind != SemanticActionKind::McpClientReceive
        }),
        "split HTTP/2 SSE event must wait for the terminating blank line"
    );

    let output = runtime.observe_payload_segment(&remote_mcp_http2_response_raw_segment(
        client,
        2,
        remote_mcp_http2_data(5, &body.as_bytes()[split_at..]),
    ));

    let completed = mcp_action(&output);
    let response = action_of_kind(&output, SemanticActionKind::McpResponse);
    let client_receive = action_of_kind(&output, SemanticActionKind::McpClientReceive);
    assert_eq!(completed.status, SemanticActionStatus::Success);
    assert_eq!(response.status, SemanticActionStatus::Success);
    assert_eq!(client_receive.status, SemanticActionStatus::Success);
    assert_eq!(attr(response, "http.request.stream_id"), None);
    assert_eq!(attr(response, "http.response.stream_id"), Some("5"));
    assert_eq!(attr(client_receive, "http.request.stream_id"), None);
    assert_eq!(attr(client_receive, "http.response.stream_id"), Some("5"));
    assert_eq!(
        attr(client_receive, "mcp.message.direction"),
        Some("inbound")
    );
}

#[test]
fn remote_http_header_delimited_sse_response_completes_matching_tool_call() {
    let mut runtime = runtime();
    let client = ProcessIdentity::new(5107, 9107, 10107);

    runtime.observe_payload_segment(&remote_mcp_request_segment(
        client.clone(),
        0,
        "remote.example.test",
        "/mcp",
        None,
        &tools_call_request(88, "actrail_remote_probe.emit_remote_marker"),
    ));
    let headers = runtime.observe_payload_segment(&remote_mcp_response_raw_segment(
        client.clone(),
        1,
        b"HTTP/1.0 200 OK\r\nContent-Type: text/event-stream\r\n\r\n".to_vec(),
    ));
    assert!(
        headers
            .actions
            .iter()
            .all(|action| action.kind != SemanticActionKind::McpToolCall),
        "header-only SSE response must wait for response body"
    );

    let ping = format!("event: message\ndata: {}\n\n", ping_request("server-ping"));
    let ping_output = runtime.observe_payload_segment(&remote_mcp_response_raw_segment(
        client.clone(),
        2,
        ping.into_bytes(),
    ));
    let ping_receive = action_of_kind(&ping_output, SemanticActionKind::McpClientReceive);
    assert_eq!(attr(ping_receive, "mcp.message.id"), Some("server-ping"));
    assert_eq!(attr(ping_receive, "mcp.message.method"), Some("ping"));
    assert_eq!(attr(ping_receive, "mcp.message.direction"), Some("inbound"));
    assert_eq!(attr(ping_receive, "mcp.message.sequence"), Some("2"));
    assert_eq!(attr(ping_receive, "mcp.exchange.index"), Some("2"));
    assert_eq!(attr(ping_receive, "mcp.tool_call.request_id"), Some("88"));
    assert_eq!(attr(ping_receive, "http.request.method"), None);
    assert_eq!(
        attr(ping_receive, "http.response.protocol"),
        Some("http/1.0")
    );
    assert_eq!(attr(ping_receive, "http.response.status_code"), Some("200"));
    assert_eq!(attr(ping_receive, "http.response.reason"), Some("OK"));
    assert!(ping_receive.evidence.iter().any(|evidence| {
        evidence.kind == SemanticEvidenceKind::PayloadSegment
            && evidence.role == "mcp.client_receive.payload"
    }));
    assert!(
        ping_output.actions.iter().all(|action| {
            action.kind != SemanticActionKind::McpToolCall
                && action.kind != SemanticActionKind::McpResponse
        }),
        "unmatched server ping SSE event must not close the tool call"
    );

    let ping_response_output = runtime.observe_payload_segment(&remote_mcp_request_segment(
        client.clone(),
        3,
        "remote.example.test",
        "/mcp",
        None,
        &ping_response("server-ping"),
    ));
    let ping_send = action_of_kind(&ping_response_output, SemanticActionKind::McpClientSend);
    assert_eq!(attr(ping_send, "mcp.message.id"), Some("server-ping"));
    assert_eq!(attr(ping_send, "mcp.message.direction"), Some("outbound"));
    assert_eq!(attr(ping_send, "mcp.message.sequence"), Some("2"));
    assert_eq!(attr(ping_send, "mcp.exchange.index"), Some("2"));
    assert_eq!(attr(ping_send, "mcp.tool_call.request_id"), Some("88"));
    assert!(ping_send.evidence.iter().any(|evidence| {
        evidence.kind == SemanticEvidenceKind::PayloadSegment
            && evidence.role == "mcp.client_send.payload"
    }));

    let body = format!("event: message\ndata: {}\n\n", tool_success_response(88));
    let output = runtime.observe_payload_segment(&remote_mcp_response_raw_segment(
        client,
        4,
        body.into_bytes(),
    ));

    let completed = mcp_action(&output);
    let response = action_of_kind(&output, SemanticActionKind::McpResponse);
    let client_receive = action_of_kind(&output, SemanticActionKind::McpClientReceive);
    assert_eq!(completed.status, SemanticActionStatus::Success);
    assert_eq!(response.status, SemanticActionStatus::Success);
    assert_eq!(client_receive.status, SemanticActionStatus::Success);
    assert_eq!(attr(response, "mcp.transport"), Some("streamable_http"));
    assert_eq!(attr(response, "http.request.method"), None);
    assert_eq!(attr(response, "http.response.protocol"), Some("http/1.0"));
    assert_eq!(attr(response, "http.response.status_code"), Some("200"));
    assert_eq!(attr(response, "http.response.reason"), Some("OK"));
    assert_eq!(attr(client_receive, "http.request.method"), None);
    assert_eq!(
        attr(client_receive, "http.response.protocol"),
        Some("http/1.0")
    );
    assert_eq!(
        attr(client_receive, "http.response.status_code"),
        Some("200")
    );
    assert_eq!(attr(client_receive, "http.response.reason"), Some("OK"));
    assert!(client_receive.evidence.iter().any(|evidence| {
        evidence.kind == SemanticEvidenceKind::PayloadSegment
            && evidence.role == "mcp.client_receive.payload"
    }));
    assert!(
        client_receive.evidence.iter().all(|evidence| {
            evidence.kind != SemanticEvidenceKind::PayloadSegment || evidence.id != 1502
        }),
        "final tool-result client_receive must not retain the earlier server-ping SSE payload"
    );
    assert!(
        client_receive.evidence.iter().any(|evidence| {
            evidence.kind == SemanticEvidenceKind::PayloadSegment
                && evidence.id == 1501
                && evidence.role == "mcp.client_receive.payload"
        }),
        "final tool-result client_receive should retain the SSE HTTP response head"
    );
    assert!(
        client_receive.evidence.iter().any(|evidence| {
            evidence.kind == SemanticEvidenceKind::PayloadSegment
                && evidence.id == 1504
                && evidence.role == "mcp.client_receive.payload"
        }),
        "final tool-result client_receive should retain the result SSE payload"
    );

    let finalized = runtime.finalize_trace(TRACE_ID, UNIX_EPOCH + Duration::from_secs(10));
    assert!(
        finalized.actions.iter().all(|action| {
            action.kind != SemanticActionKind::McpToolCall
                || attr(action, "actrail.action.finalized_on_trace_close") != Some("true")
        }),
        "completed remote MCP tool call must not be finalized as a trace-close error"
    );
}

#[test]
fn remote_http_ping_response_on_separate_stream_stays_under_active_tool_call() {
    let mut runtime = runtime();
    let client = ProcessIdentity::new(5108, 9108, 10108);

    runtime.observe_payload_segment(&remote_mcp_request_segment(
        client.clone(),
        0,
        "remote.example.test",
        "/mcp",
        None,
        &tools_call_request(89, "actrail_remote_probe.emit_remote_marker"),
    ));
    runtime.observe_payload_segment(&remote_mcp_response_raw_segment(
        client.clone(),
        1,
        b"HTTP/1.0 200 OK\r\nContent-Type: text/event-stream\r\n\r\n".to_vec(),
    ));

    let ping = format!("event: message\ndata: {}\n\n", ping_request("server-ping"));
    let ping_output = runtime.observe_payload_segment(&remote_mcp_response_raw_segment(
        client.clone(),
        2,
        ping.into_bytes(),
    ));
    let ping_receive = action_of_kind(&ping_output, SemanticActionKind::McpClientReceive);
    assert_eq!(attr(ping_receive, "mcp.message.id"), Some("server-ping"));
    assert_eq!(attr(ping_receive, "mcp.exchange.index"), Some("2"));

    let ping_response_output =
        runtime.observe_payload_segment(&remote_mcp_request_segment_with_stream(
            client.clone(),
            3,
            "remote-mcp-reply",
            "remote.example.test",
            "/mcp",
            None,
            &ping_response("server-ping"),
        ));
    let ping_send = action_of_kind(&ping_response_output, SemanticActionKind::McpClientSend);
    assert_eq!(attr(ping_send, "mcp.message.id"), Some("server-ping"));
    assert_eq!(attr(ping_send, "mcp.message.direction"), Some("outbound"));
    assert_eq!(attr(ping_send, "mcp.message.sequence"), Some("2"));
    assert_eq!(attr(ping_send, "mcp.exchange.index"), Some("2"));
    assert_eq!(attr(ping_send, "mcp.tool_call.request_id"), Some("89"));

    let body = format!("event: message\ndata: {}\n\n", tool_success_response(89));
    let output = runtime.observe_payload_segment(&remote_mcp_response_raw_segment(
        client,
        4,
        body.into_bytes(),
    ));

    let completed = mcp_action(&output);
    assert_eq!(completed.status, SemanticActionStatus::Success);
}

#[test]
fn remote_http_stale_request_context_does_not_steal_active_tool_call_server_request() {
    let mut runtime = runtime();
    let client = ProcessIdentity::new(5112, 9112, 10112);

    runtime.observe_payload_segment(&remote_mcp_request_segment(
        client.clone(),
        0,
        "remote.example.test",
        "/stale",
        None,
        &initialize_request("stale-init"),
    ));
    runtime.observe_payload_segment(&remote_mcp_request_segment(
        client.clone(),
        1,
        "remote.example.test",
        "/mcp",
        None,
        &tools_call_request(95, "actrail_remote_probe.emit_remote_marker"),
    ));
    runtime.observe_payload_segment(&remote_mcp_response_raw_segment(
        client.clone(),
        2,
        b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\n".to_vec(),
    ));

    let ping = format!("event: message\ndata: {}\n\n", ping_request("server-ping"));
    let ping_output = runtime.observe_payload_segment(&remote_mcp_response_raw_segment(
        client.clone(),
        3,
        ping.into_bytes(),
    ));
    let ping_receive = action_of_kind(&ping_output, SemanticActionKind::McpClientReceive);
    assert_eq!(attr(ping_receive, "mcp.message.id"), Some("server-ping"));
    assert_eq!(attr(ping_receive, "mcp.tool_call.request_id"), Some("95"));
    assert_eq!(attr(ping_receive, "url.path"), Some("/mcp"));

    let body = format!("event: message\ndata: {}\n\n", tool_success_response(95));
    let output = runtime.observe_payload_segment(&remote_mcp_response_raw_segment(
        client,
        4,
        body.into_bytes(),
    ));
    let completed = mcp_action(&output);
    assert_eq!(completed.status, SemanticActionStatus::Success);
}

#[test]
fn remote_http_split_sse_block_waits_for_complete_event() {
    let mut runtime = runtime();
    let client = ProcessIdentity::new(5108, 9108, 10108);

    runtime.observe_payload_segment(&remote_mcp_request_segment(
        client.clone(),
        0,
        "remote.example.test",
        "/mcp",
        None,
        &tools_call_request(89, "actrail_remote_probe.emit_remote_marker"),
    ));
    runtime.observe_payload_segment(&remote_mcp_response_raw_segment(
        client.clone(),
        1,
        b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\n".to_vec(),
    ));

    let event = format!("event: message\ndata: {}\n\n", tool_success_response(89));
    let split_at = event.len() - 1;
    let partial = runtime.observe_payload_segment(&remote_mcp_response_raw_segment(
        client.clone(),
        2,
        event[..split_at].as_bytes().to_vec(),
    ));
    assert!(
        partial.actions.iter().all(|action| {
            action.kind != SemanticActionKind::McpToolCall
                && action.kind != SemanticActionKind::McpResponse
                && action.kind != SemanticActionKind::McpClientReceive
        }),
        "split SSE event must wait for the terminating blank line"
    );

    let output = runtime.observe_payload_segment(&remote_mcp_response_raw_segment(
        client,
        3,
        event[split_at..].as_bytes().to_vec(),
    ));

    let completed = mcp_action(&output);
    let response = action_of_kind(&output, SemanticActionKind::McpResponse);
    let client_receive = action_of_kind(&output, SemanticActionKind::McpClientReceive);
    assert_eq!(completed.status, SemanticActionStatus::Success);
    assert_eq!(response.status, SemanticActionStatus::Success);
    assert_eq!(client_receive.status, SemanticActionStatus::Success);
}

#[test]
fn oversized_incomplete_sse_event_does_not_close_open_sse_context() {
    let mut runtime = runtime_with_mcp_parse_buffer_max_bytes(TEST_MCP_PARSE_BUFFER_MAX_BYTES);
    let client = ProcessIdentity::new(5181, 9181, 10181);

    runtime.observe_payload_segment(&remote_mcp_request_segment(
        client.clone(),
        0,
        "h",
        "/m",
        None,
        &tools_call_request(902, "t"),
    ));
    runtime.observe_payload_segment(&remote_mcp_response_raw_segment(
        client.clone(),
        1,
        b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\n".to_vec(),
    ));

    let mut oversized_event = b"event: message\ndata: ".to_vec();
    oversized_event.extend(vec![b'x'; TEST_MCP_PARSE_BUFFER_MAX_BYTES as usize]);
    let oversized = runtime.observe_payload_segment(&remote_mcp_response_raw_segment(
        client.clone(),
        2,
        oversized_event,
    ));
    assert_no_mcp_projection(&oversized);

    let complete_event = format!("event: message\ndata: {}\n\n", tool_success_response(902));
    let output = runtime.observe_payload_segment(&remote_mcp_response_raw_segment(
        client,
        3,
        complete_event.into_bytes(),
    ));

    let completed = mcp_action(&output);
    let response = action_of_kind(&output, SemanticActionKind::McpResponse);
    let client_receive = action_of_kind(&output, SemanticActionKind::McpClientReceive);
    assert_eq!(completed.status, SemanticActionStatus::Success);
    assert_eq!(response.status, SemanticActionStatus::Success);
    assert_eq!(client_receive.status, SemanticActionStatus::Success);
}

#[test]
fn remote_http_endpoint_metadata_does_not_leak_between_endpoints() {
    let mut runtime = runtime();
    let client = ProcessIdentity::new(5104, 9104, 10104);

    runtime.observe_payload_segment(&remote_mcp_request_segment_with_stream(
        client.clone(),
        0,
        "remote-a",
        "remote.example.test",
        "/mcp-a",
        None,
        &initialize_request("init-a"),
    ));
    runtime.observe_payload_segment(&remote_mcp_response_segment_with_stream(
        client.clone(),
        1,
        "remote-a",
        Some("application/json"),
        None,
        &initialize_response_with_id("\"init-a\"", "server_a"),
    ));
    runtime.observe_payload_segment(&remote_mcp_request_segment_with_stream(
        client.clone(),
        2,
        "remote-b",
        "remote.example.test",
        "/mcp-b",
        None,
        &initialize_request("init-b"),
    ));
    runtime.observe_payload_segment(&remote_mcp_response_segment_with_stream(
        client.clone(),
        3,
        "remote-b",
        Some("application/json"),
        None,
        &initialize_response_with_id("\"init-b\"", "server_b"),
    ));

    let output = runtime.observe_payload_segment(&remote_mcp_request_segment_with_stream(
        client,
        4,
        "remote-b",
        "remote.example.test",
        "/mcp-b",
        None,
        &tools_call_request(86, "emit_remote_marker"),
    ));
    let tool_call = mcp_action(&output);

    assert_eq!(attr(tool_call, "mcp.server.name"), Some("server_b"));
    assert_ne!(attr(tool_call, "mcp.server.name"), Some("server_a"));
    assert_eq!(attr(tool_call, "url.path"), Some("/mcp-b"));
}

#[test]
fn non_mcp_jsonrpc_http_payload_does_not_emit_tool_call() {
    let mut runtime = runtime();
    let client = ProcessIdentity::new(5105, 9105, 10105);

    let output = runtime.observe_payload_segment(&remote_mcp_request_segment(
        client,
        0,
        "api.example.test",
        "/rpc",
        None,
        r#"{"jsonrpc":"2.0","id":1,"method":"resources/read","params":{"uri":"file:///tmp/x"}}"#,
    ));

    assert!(output.actions.iter().all(|action| {
        action.kind != SemanticActionKind::McpToolCall
            && action.kind != SemanticActionKind::McpRequest
            && action.kind != SemanticActionKind::McpResponse
            && action.kind != SemanticActionKind::McpStdin
            && action.kind != SemanticActionKind::McpStdout
    }));
}

#[test]
fn mcp_tool_call_does_not_overwrite_agent_command_invocation_kind() {
    let mut runtime = runtime();
    let server = ProcessIdentity::new(5001, 9001, 10001);

    runtime.observe_event(&exec_event(
        EventId::new(690),
        server.clone(),
        None,
        "/root/.claude/local/claude",
    ));
    runtime.observe_payload_segment(&llm_payload_segment(server.clone()));
    let request_output = runtime.observe_payload_segment(&mcp_stdin_segment(
        server.clone(),
        0,
        line(tools_call_request(53, "emit_probe")),
    ));

    let command = command_action(&request_output);
    assert_eq!(command.process, server);
    assert_eq!(attr(command, "invocation.kind"), Some("agent"));
    assert!(command.evidence.iter().any(|evidence| {
        evidence.kind == SemanticEvidenceKind::PayloadSegment
            && evidence.role == "mcp.tool_call.payload"
    }));
}

fn mcp_action(output: &LiveSemanticActionOutput) -> &SemanticAction {
    output
        .actions
        .iter()
        .find(|action| action.kind.as_str() == "mcp.tool_call")
        .expect("expected mcp.tool_call action")
}

fn action_of_kind(output: &LiveSemanticActionOutput, kind: SemanticActionKind) -> &SemanticAction {
    output
        .actions
        .iter()
        .find(|action| action.kind == kind)
        .unwrap_or_else(|| panic!("expected {} action", kind.as_str()))
}

fn command_action(output: &LiveSemanticActionOutput) -> &SemanticAction {
    output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::CommandInvocation)
        .expect("expected command.invocation action")
}

fn assert_no_mcp_projection(output: &LiveSemanticActionOutput) {
    assert!(
        output.actions.iter().all(|action| {
            !matches!(
                action.kind,
                SemanticActionKind::McpToolCall
                    | SemanticActionKind::McpRequest
                    | SemanticActionKind::McpResponse
                    | SemanticActionKind::McpStdin
                    | SemanticActionKind::McpStdout
                    | SemanticActionKind::McpClientSend
                    | SemanticActionKind::McpClientReceive
            )
        }),
        "malformed partial payload should not project MCP actions"
    );
}

fn attr<'a>(action: &'a SemanticAction, key: &str) -> Option<&'a str> {
    action.attributes.get(key).map(String::as_str)
}

fn mcp_stdin_segment(
    process: ProcessIdentity,
    index: u64,
    bytes: Vec<u8>,
) -> model_core::payload::PayloadSegment {
    mcp_segment(process, PayloadDirection::Inbound, index, bytes)
}

fn mcp_stdout_segment(
    process: ProcessIdentity,
    index: u64,
    bytes: Vec<u8>,
) -> model_core::payload::PayloadSegment {
    mcp_segment(process, PayloadDirection::Outbound, index, bytes)
}

fn mcp_stdin_segment_with_stream(
    process: ProcessIdentity,
    index: u64,
    stream_key: &str,
    bytes: Vec<u8>,
) -> model_core::payload::PayloadSegment {
    mcp_segment_with_stream(process, PayloadDirection::Inbound, index, stream_key, bytes)
}

fn mcp_stdout_segment_with_stream(
    process: ProcessIdentity,
    index: u64,
    stream_key: &str,
    bytes: Vec<u8>,
) -> model_core::payload::PayloadSegment {
    mcp_segment_with_stream(
        process,
        PayloadDirection::Outbound,
        index,
        stream_key,
        bytes,
    )
}

fn mcp_segment(
    process: ProcessIdentity,
    direction: PayloadDirection,
    index: u64,
    bytes: Vec<u8>,
) -> model_core::payload::PayloadSegment {
    mcp_segment_with_stream(process, direction, index, MCP_STREAM, bytes)
}

fn mcp_segment_with_stream(
    process: ProcessIdentity,
    direction: PayloadDirection,
    index: u64,
    stream_key: &str,
    bytes: Vec<u8>,
) -> model_core::payload::PayloadSegment {
    stdio_payload_segment_with_bytes(
        process,
        direction,
        PayloadSegmentId::new(600 + index),
        700 + index,
        index,
        stream_key,
        bytes,
    )
}

fn remote_mcp_request_segment(
    process: ProcessIdentity,
    index: u64,
    authority: &str,
    path: &str,
    session_id: Option<&str>,
    body: &str,
) -> model_core::payload::PayloadSegment {
    remote_mcp_request_segment_with_stream(
        process,
        index,
        REMOTE_MCP_STREAM,
        authority,
        path,
        session_id,
        body,
    )
}

fn remote_mcp_request_segment_with_stream(
    process: ProcessIdentity,
    index: u64,
    stream_key: &str,
    authority: &str,
    path: &str,
    session_id: Option<&str>,
    body: &str,
) -> model_core::payload::PayloadSegment {
    let session = session_id
        .map(|id| format!("MCP-Session-Id: {id}\r\n"))
        .unwrap_or_default();
    let bytes = format!(
        "POST {path} HTTP/1.1\r\nHost: {authority}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\n{session}Content-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .into_bytes();
    let mut segment = outbound_http1_payload_segment_with_bytes(
        process,
        PayloadSegmentId::new(1200 + index),
        1300 + index,
        index,
        bytes,
    );
    segment.stream_key = model_core::payload::PayloadStreamKey::new(stream_key);
    segment
}

fn remote_mcp_response_segment(
    process: ProcessIdentity,
    index: u64,
    content_type: Option<&str>,
    session_id: Option<&str>,
    body: &str,
) -> model_core::payload::PayloadSegment {
    remote_mcp_response_segment_with_stream(
        process,
        index,
        REMOTE_MCP_STREAM,
        content_type,
        session_id,
        body,
    )
}

fn remote_mcp_response_segment_with_stream(
    process: ProcessIdentity,
    index: u64,
    stream_key: &str,
    content_type: Option<&str>,
    session_id: Option<&str>,
    body: &str,
) -> model_core::payload::PayloadSegment {
    let content_type = content_type
        .map(|value| format!("Content-Type: {value}\r\n"))
        .unwrap_or_default();
    let session = session_id
        .map(|id| format!("MCP-Session-Id: {id}\r\n"))
        .unwrap_or_default();
    let bytes = format!(
        "HTTP/1.1 200 OK\r\n{content_type}{session}Content-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .into_bytes();
    let mut segment = llm_response_payload_segment(
        process,
        PayloadSegmentId::new(1400 + index),
        1500 + index,
        index,
        bytes,
    );
    segment.stream_key = model_core::payload::PayloadStreamKey::new(stream_key);
    segment
}

fn remote_mcp_response_raw_segment(
    process: ProcessIdentity,
    index: u64,
    bytes: Vec<u8>,
) -> model_core::payload::PayloadSegment {
    let mut segment = llm_response_payload_segment(
        process,
        PayloadSegmentId::new(1500 + index),
        1600 + index,
        index,
        bytes,
    );
    segment.stream_key = model_core::payload::PayloadStreamKey::new(REMOTE_MCP_STREAM);
    segment
}

fn remote_mcp_http2_request_raw_segment(
    process: ProcessIdentity,
    index: u64,
    bytes: Vec<u8>,
) -> model_core::payload::PayloadSegment {
    let mut segment = outbound_http1_payload_segment_with_bytes(
        process,
        PayloadSegmentId::new(1600 + index),
        1700 + index,
        index,
        bytes,
    );
    segment.protocol_hint = Some("h2".to_string());
    segment.stream_key = model_core::payload::PayloadStreamKey::new(REMOTE_MCP_STREAM);
    segment
}

fn remote_mcp_http2_response_raw_segment(
    process: ProcessIdentity,
    index: u64,
    bytes: Vec<u8>,
) -> model_core::payload::PayloadSegment {
    let mut segment = llm_response_payload_segment(
        process,
        PayloadSegmentId::new(1700 + index),
        1800 + index,
        index,
        bytes,
    );
    segment.protocol_hint = Some("h2".to_string());
    segment.stream_key = model_core::payload::PayloadStreamKey::new(REMOTE_MCP_STREAM);
    segment
}

fn set_segment_bytes(segment: &mut model_core::payload::PayloadSegment, bytes: Vec<u8>) {
    let size = bytes.len() as u64;
    segment.bytes = bytes;
    segment.original_size = size;
    segment.captured_size = size;
    segment.operation_original_size = size;
    segment.operation_captured_size = size;
}

fn remote_mcp_http2_requests<const N: usize>(requests: [(u32, String); N]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for (stream_id, _) in &requests {
        bytes.extend(http2_frame(HTTP2_HEADERS_FRAME_TYPE, *stream_id, b"\x83"));
    }
    for (stream_id, body) in requests {
        bytes.extend(http2_frame(
            HTTP2_DATA_FRAME_TYPE,
            stream_id,
            body.as_bytes(),
        ));
    }
    bytes
}

fn remote_mcp_http2_response(stream_id: u32, body: &[u8]) -> Vec<u8> {
    let mut bytes = http2_frame(HTTP2_HEADERS_FRAME_TYPE, stream_id, b"\x88");
    bytes.extend(http2_frame(HTTP2_DATA_FRAME_TYPE, stream_id, body));
    bytes
}

fn remote_mcp_http2_data(stream_id: u32, body: &[u8]) -> Vec<u8> {
    http2_frame(HTTP2_DATA_FRAME_TYPE, stream_id, body)
}

fn http2_frame(frame_type: u8, stream_id: u32, payload: &[u8]) -> Vec<u8> {
    assert!(payload.len() <= HTTP2_MAX_FRAME_LENGTH_FIELD);
    assert!(stream_id != 0);
    let length = payload.len();
    let mut frame = Vec::with_capacity(HTTP2_FRAME_HEADER_BYTES + length);
    frame.push(((length >> 16) & HTTP2_LENGTH_BYTE_MASK) as u8);
    frame.push(((length >> 8) & HTTP2_LENGTH_BYTE_MASK) as u8);
    frame.push((length & HTTP2_LENGTH_BYTE_MASK) as u8);
    frame.push(frame_type);
    frame.push(0);
    frame.extend_from_slice(&(stream_id & HTTP2_RESERVED_STREAM_ID_MASK).to_be_bytes());
    frame.extend_from_slice(payload);
    frame
}

fn line(json: impl AsRef<str>) -> Vec<u8> {
    format!("{}\n", json.as_ref()).into_bytes()
}

fn framed(json: impl AsRef<str>) -> Vec<u8> {
    let json = json.as_ref();
    format!("Content-Length: {}\r\n\r\n{json}", json.len()).into_bytes()
}

fn llm_response_with_tool_call(
    process: ProcessIdentity,
    tool_name: &str,
) -> model_core::payload::PayloadSegment {
    let sse = format!(
        r#"data: {{"model":"claude-sonnet-4","choices":[{{"delta":{{"tool_calls":[{{"index":0,"id":"call_mcp_1","type":"function","function":{{"name":"{tool_name}","arguments":"{{}}"}}}}]}}}}]}}

data: [DONE]

"#
    );
    let bytes = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\r\n{}",
        sse.len(),
        sse
    )
    .into_bytes();
    llm_response_payload_segment(process, PayloadSegmentId::new(900), 901, 902, bytes)
}

fn initialize_response(server_name: &str) -> String {
    format!(
        r#"{{"jsonrpc":"2.0","id":1,"result":{{"protocolVersion":"2024-11-05","serverInfo":{{"name":"{server_name}","version":"1.0.0"}},"capabilities":{{}}}}}}"#
    )
}

fn initialize_request(id: &str) -> String {
    format!(
        r#"{{"jsonrpc":"2.0","id":"{id}","method":"initialize","params":{{"protocolVersion":"2025-11-25","capabilities":{{}},"clientInfo":{{"name":"actrail-test","version":"1.0.0"}}}}}}"#
    )
}

fn initialize_response_with_id(id: &str, server_name: &str) -> String {
    format!(
        r#"{{"jsonrpc":"2.0","id":{id},"result":{{"protocolVersion":"2025-11-25","serverInfo":{{"name":"{server_name}","version":"1.0.0"}},"capabilities":{{}}}}}}"#
    )
}

fn tools_list_response(id: u64, names: &[&str]) -> String {
    let tools = names
        .iter()
        .map(|name| format!(r#"{{"name":"{name}","description":"test tool"}}"#))
        .collect::<Vec<_>>()
        .join(",");
    format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{{"tools":[{tools}]}}}}"#)
}

fn tools_call_request(id: u64, name: &str) -> String {
    format!(
        r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/call","params":{{"name":"{name}","arguments":{{"message":"hello"}}}}}}"#
    )
}

fn tool_success_response(id: u64) -> String {
    format!(
        r#"{{"jsonrpc":"2.0","id":{id},"result":{{"content":[{{"type":"text","text":"ok"}}],"isError":false}}}}"#
    )
}

fn tool_error_response(id: u64) -> String {
    format!(r#"{{"jsonrpc":"2.0","id":{id},"error":{{"code":-32603,"message":"boom"}}}}"#)
}

fn tool_result_is_error_response(id: u64) -> String {
    format!(
        r#"{{"jsonrpc":"2.0","id":{id},"result":{{"content":[{{"type":"text","text":"boom"}}],"isError":true}}}}"#
    )
}

fn ping_request(id: &str) -> String {
    format!(r#"{{"jsonrpc":"2.0","id":"{id}","method":"ping"}}"#)
}

fn ping_response(id: &str) -> String {
    format!(r#"{{"jsonrpc":"2.0","id":"{id}","result":{{}}}}"#)
}
