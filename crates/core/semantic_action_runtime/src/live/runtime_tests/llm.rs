use model_core::process::ProcessIdentity;
use semantic_action::{
    SemanticActionCompleteness, SemanticActionKind, SemanticActionLinkRole, SemanticActionStatus,
};
use serde_json::Value;

use super::test_support::*;

#[test]
fn llm_response_updates_one_action_with_multiple_sse_segments() {
    let mut runtime = runtime();
    let agent = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);

    let mut first_response = None;
    for (index, bytes) in [
        b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n"
            .to_vec(),
        http_chunk_prefix(
            "data: {\"model\":\"deepseek-v4-flash\",\"choices\":[{\"delta\":{\"content\":null,\"reasoning_content\":\"hel\"}}]}\n\n",
        ),
        "data: {\"model\":\"deepseek-v4-flash\",\"choices\":[{\"delta\":{\"content\":null,\"reasoning_content\":\"hel\"}}]}\n\n"
            .as_bytes()
            .to_vec(),
        b"\r\n".to_vec(),
    ]
    .into_iter()
    .enumerate()
    {
        let output = runtime.observe_payload_segment(&llm_response_payload_segment(
            agent.clone(),
            response_segment_id(index),
            response_operation_id(index),
            response_sequence(index),
            bytes,
        ));
        if first_response.is_none() {
            first_response = output
                .actions
                .into_iter()
                .find(|action| action.kind == SemanticActionKind::LlmResponse);
        }
    }

    runtime.observe_payload_segment(&llm_response_payload_segment(
        agent.clone(),
        response_segment_id(4),
        response_operation_id(4),
        response_sequence(4),
        http_chunk_prefix(
            r#"data: {"model":"deepseek-v4-flash","choices":[{"delta":{"content":null,"reasoning_content":"lo"}}]}

data: [DONE]

"#,
        ),
    ));
    let final_body_output = runtime.observe_payload_segment(&llm_response_payload_segment(
        agent.clone(),
        response_segment_id(5),
        response_operation_id(5),
        response_sequence(5),
        r#"data: {"model":"deepseek-v4-flash","choices":[{"delta":{"content":null,"reasoning_content":"lo"}}]}

data: [DONE]

"#
        .as_bytes()
        .to_vec(),
    ));
    let terminal_chunk_output = runtime.observe_payload_segment(&llm_response_payload_segment(
        agent,
        response_segment_id(6),
        response_operation_id(6),
        response_sequence(6),
        b"\r\n0\r\n\r\n".to_vec(),
    ));
    let first_response = first_response
        .as_ref()
        .expect("first SSE segment should create an in-progress llm.response");
    let second_response = final_body_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmResponse)
        .expect("second SSE segment should update the same llm.response");
    assert_eq!(second_response.action_id, first_response.action_id);
    let terminal_response = terminal_chunk_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmResponse)
        .expect("chunk terminator should finalize the same llm.response");
    assert_eq!(terminal_response.action_id, first_response.action_id);
    assert_eq!(second_response.status, SemanticActionStatus::Success);
    assert_eq!(
        second_response.completeness,
        SemanticActionCompleteness::Complete
    );
    assert_eq!(
        second_response
            .attributes
            .get("llm.response.output_text")
            .map(String::as_str),
        Some("hello")
    );
    assert!(second_response.evidence.iter().any(|evidence| {
        evidence.id == RESPONSE_FIRST_SEGMENT_ID.get() && evidence.role == "llm.response.payload"
    }));
    assert!(second_response.evidence.iter().any(|evidence| {
        evidence.id == response_segment_id(5).get() && evidence.role == "llm.response.payload"
    }));
}

#[test]
fn llm_response_stream_state_keeps_chunked_boundary_after_done() {
    let mut runtime = runtime();
    let agent = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);

    let first_body = concat!(
        r#"data: {"model":"deepseek-v4-flash","choices":[{"delta":{"content":"first"}}]}"#,
        "\n\n",
        "data: [DONE]\n\n",
    );
    let first_segments = [
        b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n"
            .to_vec(),
        http_chunk_prefix(first_body),
        first_body.as_bytes().to_vec(),
        b"\r\n0\r\n\r\n".to_vec(),
    ];
    let mut first_response_id = None;
    for (index, bytes) in first_segments.into_iter().enumerate() {
        let output = runtime.observe_payload_segment(&llm_response_payload_segment(
            agent.clone(),
            response_segment_id(index),
            response_operation_id(index),
            response_sequence(index),
            bytes,
        ));
        if let Some(response) = output
            .actions
            .iter()
            .find(|action| action.kind == SemanticActionKind::LlmResponse)
        {
            first_response_id = Some(response.action_id.clone());
        }
    }
    let first_response_id = first_response_id.expect("first response should project");

    let second_body = concat!(
        r#"data: {"model":"deepseek-v4-flash","choices":[{"delta":{"content":"second"}}]}"#,
        "\n\n",
        "data: [DONE]\n\n",
    );
    let second_bytes = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{}\r\n0\r\n\r\n",
        second_body.len(),
        second_body,
    )
    .into_bytes();
    let second_output = runtime.observe_payload_segment(&llm_response_payload_segment(
        agent,
        response_segment_id(4),
        response_operation_id(4),
        response_sequence(4),
        second_bytes,
    ));
    let second_response = second_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmResponse)
        .expect("second response should project independently");

    assert_ne!(second_response.action_id, first_response_id);
    assert!(!second_response.action_id.contains(":raw:llm.response"));
    assert_eq!(
        second_response
            .attributes
            .get("llm.response.output_text")
            .map(String::as_str),
        Some("second")
    );
}

#[test]
fn llm_response_suppresses_repeated_in_progress_updates() {
    let mut runtime = runtime();
    let agent = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);

    for (index, bytes) in [
        b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n"
            .to_vec(),
        http_chunk_prefix(
            "data: {\"model\":\"deepseek-v4-flash\",\"choices\":[{\"delta\":{\"reasoning_content\":\"hel\"}}]}\n\n",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        runtime.observe_payload_segment(&llm_response_payload_segment(
            agent.clone(),
            response_segment_id(index),
            response_operation_id(index),
            response_sequence(index),
            bytes,
        ));
    }

    let first = runtime.observe_payload_segment(&llm_response_payload_segment(
        agent.clone(),
        response_segment_id(2),
        response_operation_id(2),
        response_sequence(2),
        "data: {\"model\":\"deepseek-v4-flash\",\"choices\":[{\"delta\":{\"reasoning_content\":\"hel\"}}]}\n\n"
            .as_bytes()
            .to_vec(),
    ));
    runtime.observe_payload_segment(&llm_response_payload_segment(
        agent.clone(),
        response_segment_id(3),
        response_operation_id(3),
        response_sequence(3),
        b"\r\n".to_vec(),
    ));
    runtime.observe_payload_segment(&llm_response_payload_segment(
        agent.clone(),
        response_segment_id(4),
        response_operation_id(4),
        response_sequence(4),
        http_chunk_prefix(
            r#"data: {"model":"deepseek-v4-flash","choices":[{"delta":{"reasoning_content":"lo"}}]}

"#,
        ),
    ));
    let second = runtime.observe_payload_segment(&llm_response_payload_segment(
        agent,
        response_segment_id(5),
        response_operation_id(5),
        response_sequence(5),
        r#"data: {"model":"deepseek-v4-flash","choices":[{"delta":{"reasoning_content":"lo"}}]}

"#
        .as_bytes()
        .to_vec(),
    ));

    assert!(
        first
            .actions
            .iter()
            .any(|action| action.kind == SemanticActionKind::LlmResponse)
    );
    assert!(
        second
            .actions
            .iter()
            .all(|action| action.kind != SemanticActionKind::LlmResponse)
    );
}

#[test]
fn llm_response_stream_state_evicts_completed_messages() {
    let mut runtime = runtime();
    let agent = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);

    let first_body = r#"{"model":"deepseek-chat","choices":[{"message":{"content":"first"}}]}"#;
    let first_bytes = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
        first_body.len(),
        first_body
    )
    .into_bytes();
    let first_output = runtime.observe_payload_segment(&llm_response_payload_segment(
        agent.clone(),
        response_segment_id(0),
        response_operation_id(0),
        response_sequence(0),
        first_bytes,
    ));
    let first_response = first_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmResponse)
        .expect("first response should project an llm.response");

    let second_body = r#"{"model":"deepseek-chat","choices":[{"message":{"content":"second"}}]}"#;
    let second_bytes = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
        second_body.len(),
        second_body
    )
    .into_bytes();
    let second_output = runtime.observe_payload_segment(&llm_response_payload_segment(
        agent,
        response_segment_id(1),
        response_operation_id(1),
        response_sequence(1),
        second_bytes,
    ));
    let second_response = second_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmResponse)
        .expect("second response should project an llm.response");

    assert_ne!(second_response.action_id, first_response.action_id);
    assert_eq!(
        second_response
            .attributes
            .get("llm.response.output_text")
            .map(String::as_str),
        Some("second")
    );
    assert!(second_response.evidence.iter().any(|evidence| {
        evidence.id == response_segment_id(1).get() && evidence.role == "llm.response.payload"
    }));
    assert!(second_response.evidence.iter().all(|evidence| {
        evidence.id != response_segment_id(0).get() || evidence.role != "llm.response.payload"
    }));
}

#[test]
fn raw_llm_responses_use_distinct_action_ids_after_evict() {
    let mut runtime = runtime();
    let agent = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);

    let first = runtime.observe_payload_segment(&llm_response_payload_segment(
        agent.clone(),
        response_segment_id(0),
        response_operation_id(0),
        response_sequence(0),
        concat!(
            r#"data: {"model":"deepseek-chat","choices":[{"delta":{"content":"first"}}]}"#,
            "\n\n",
            "data: [DONE]\n\n",
        )
        .as_bytes()
        .to_vec(),
    ));
    let first_response = first
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmResponse)
        .expect("first raw SSE response should project");

    let chunk_crlf = runtime.observe_payload_segment(&llm_response_payload_segment(
        agent.clone(),
        response_segment_id(1),
        response_operation_id(1),
        response_sequence(1),
        b"\r\n".to_vec(),
    ));
    assert!(chunk_crlf.actions.iter().all(|action| {
        action.kind != SemanticActionKind::LlmResponse
            && action.kind != SemanticActionKind::SseStream
            && action.kind != SemanticActionKind::SseEvent
    }));
    let zero_chunk = runtime.observe_payload_segment(&llm_response_payload_segment(
        agent.clone(),
        response_segment_id(2),
        response_operation_id(2),
        response_sequence(2),
        b"0\r\n\r\n".to_vec(),
    ));
    assert!(zero_chunk.actions.iter().all(|action| {
        action.kind != SemanticActionKind::LlmResponse
            && action.kind != SemanticActionKind::SseStream
            && action.kind != SemanticActionKind::SseEvent
    }));

    let second = runtime.observe_payload_segment(&llm_response_payload_segment(
        agent,
        response_segment_id(3),
        response_operation_id(3),
        response_sequence(3),
        concat!(
            r#"data: {"model":"deepseek-chat","choices":[{"delta":{"content":"second"}}]}"#,
            "\n\n",
            "data: [DONE]\n\n",
        )
        .as_bytes()
        .to_vec(),
    ));
    let second_response = second
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmResponse)
        .expect("second raw SSE response should project");

    assert_ne!(first_response.action_id, second_response.action_id);
    assert_eq!(
        second_response
            .attributes
            .get("llm.response.output_text")
            .map(String::as_str),
        Some("second")
    );
}

#[test]
fn llm_response_projects_sse_stream_and_events() {
    let mut runtime = runtime();
    let agent = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);

    let sse = concat!(
        "data: {\"model\":\"deepseek-v4-flash\",\"choices\":[{\"delta\":{\"reasoning_content\":\"thinking\"}}]}\n\n",
        r#"data: {"model":"deepseek-v4-flash","choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"bash","arguments":""}}]}}]}

"#,
        r#"data: {"model":"deepseek-v4-flash","choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"command\""}}]}}]}

"#,
        r#"data: {"model":"deepseek-v4-flash","choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":":\"pwd\"}"}}]}}]}

"#,
        "data: {\"model\":\"deepseek-v4-flash\",\"choices\":[{\"delta\":{\"content\":\"answer\"}}]}\n\n",
        "data: [DONE]\n\n"
    );
    let bytes = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\r\n{}",
        sse.len(),
        sse
    )
    .into_bytes();
    let output = runtime.observe_payload_segment(&llm_response_payload_segment(
        agent,
        RESPONSE_FIRST_SEGMENT_ID,
        RESPONSE_FIRST_OPERATION_ID,
        RESPONSE_FIRST_SEQUENCE,
        bytes,
    ));
    let response = output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmResponse)
        .expect("SSE payload should project an llm.response action");
    assert_eq!(
        response
            .attributes
            .get("llm.response.output_text")
            .map(String::as_str),
        Some("answer")
    );
    assert_eq!(
        response
            .attributes
            .get("llm.response.reasoning_text")
            .map(String::as_str),
        Some("thinking")
    );
    let tool_calls_json = response
        .attributes
        .get("llm.response.tool_calls_json")
        .expect("SSE tool call deltas should be assembled on llm.response");
    let tool_calls = serde_json::from_str::<Value>(tool_calls_json)
        .expect("assembled llm.response.tool_calls_json should be valid JSON");
    assert_eq!(tool_calls[0]["id"], "call_1");
    assert_eq!(tool_calls[0]["type"], "function");
    assert_eq!(tool_calls[0]["function"]["name"], "bash");
    assert_eq!(
        tool_calls[0]["function"]["arguments"],
        r#"{"command":"pwd"}"#
    );
    assert_eq!(
        tool_calls[0]["function"]["arguments_json"]["command"],
        "pwd"
    );

    let stream = output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::SseStream)
        .expect("SSE payload should project an sse.stream action");
    let events = output
        .actions
        .iter()
        .filter(|action| action.kind == SemanticActionKind::SseEvent)
        .collect::<Vec<_>>();
    assert_eq!(events.len(), 6);
    let event_tool_delta_json = events
        .iter()
        .find_map(|event| event.attributes.get("llm.response.delta.tool_calls_json"))
        .expect("sse.event should keep raw tool call delta JSON");
    let event_tool_delta = serde_json::from_str::<Value>(event_tool_delta_json)
        .expect("sse.event tool call delta should be valid JSON");
    assert_eq!(event_tool_delta[0]["id"], "call_1");
    assert_eq!(event_tool_delta[0]["function"]["arguments"], "");
    assert!(output.links.iter().any(|link| {
        link.role == SemanticActionLinkRole::LlmResponseSseStream
            && link.parent_action_id == response.action_id
            && link.child_action_id == stream.action_id
    }));
    assert!(events.iter().all(|event| {
        output.links.iter().any(|link| {
            link.role == SemanticActionLinkRole::SseStreamEvent
                && link.parent_action_id == stream.action_id
                && link.child_action_id == event.action_id
        })
    }));
}
