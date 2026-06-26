use model_core::event::{DomainEvent, EventPayload};
use model_core::ids::EventId;
use model_core::process::ProcessIdentity;
use semantic_action::{SemanticActionKind, attr_keys as attrs};

use super::super::test_support::*;

#[test]
fn http_response_references_preceding_http_request_action() {
    let mut runtime = runtime();
    let process = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);

    let request_output =
        runtime.observe_event(&http_request_event(HTTP_REQUEST_EVENT_ID, process.clone()));
    let request = request_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::HttpMessage)
        .expect("request event should project an http.message action");

    let response_output =
        runtime.observe_event(&http_response_event(HTTP_RESPONSE_EVENT_ID, process));
    let response = response_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::HttpMessage)
        .expect("response event should project an http.message action");

    assert_eq!(
        response
            .attributes
            .get(attrs::http_response::REQUEST_ACTION_ID)
            .map(String::as_str),
        Some(request.action_id.as_str())
    );
}

#[test]
fn http_informational_response_keeps_request_for_final_response() {
    let mut runtime = runtime();
    let process = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);

    let request_output =
        runtime.observe_event(&http_request_event(HTTP_REQUEST_EVENT_ID, process.clone()));
    let request = request_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::HttpMessage)
        .expect("request event should project an http.message action");

    let informational_output = runtime.observe_event(&http_response_event_with(
        EventId::new(1_016),
        process.clone(),
        "103",
        "Early Hints",
        RESPONSE_FIRST_SEQUENCE,
        None,
    ));
    let informational = informational_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::HttpMessage)
        .expect("informational response should project an http.message action");
    assert_eq!(
        informational
            .attributes
            .get(attrs::http_response::REQUEST_ACTION_ID)
            .map(String::as_str),
        Some(request.action_id.as_str())
    );

    let final_output = runtime.observe_event(&http_response_event_with(
        EventId::new(1_017),
        process.clone(),
        "200",
        "OK",
        response_sequence(2),
        None,
    ));
    let final_response = final_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::HttpMessage)
        .expect("final response should project an http.message action");
    assert_eq!(
        final_response
            .attributes
            .get(attrs::http_response::REQUEST_ACTION_ID)
            .map(String::as_str),
        Some(request.action_id.as_str())
    );

    let extra_output = runtime.observe_event(&http_response_event_with(
        EventId::new(1_018),
        process,
        "200",
        "OK",
        response_sequence(3),
        None,
    ));
    let extra_response = extra_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::HttpMessage)
        .expect("extra response should project an http.message action");
    assert!(
        !extra_response
            .attributes
            .contains_key(attrs::http_response::REQUEST_ACTION_ID)
    );
}

#[test]
fn http_response_matches_request_on_same_http2_stream_id() {
    let mut runtime = runtime();
    let process = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);

    let stream_three_output = runtime.observe_event(&http_request_event_with(
        EventId::new(2_015),
        process.clone(),
        response_sequence(10),
        Some("3"),
    ));
    let _stream_three = stream_three_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::HttpMessage)
        .expect("stream 3 request should project an http.message action");

    let stream_five_output = runtime.observe_event(&http_request_event_with(
        EventId::new(2_016),
        process.clone(),
        response_sequence(11),
        Some("5"),
    ));
    let stream_five = stream_five_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::HttpMessage)
        .expect("stream 5 request should project an http.message action");

    let response_output = runtime.observe_event(&http_response_event_with(
        EventId::new(2_017),
        process,
        "200",
        "OK",
        response_sequence(12),
        Some("5"),
    ));
    let response = response_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::HttpMessage)
        .expect("stream 5 response should project an http.message action");
    assert_eq!(
        response
            .attributes
            .get(attrs::http_response::REQUEST_ACTION_ID)
            .map(String::as_str),
        Some(stream_five.action_id.as_str())
    );
}

fn http_request_event_with(
    event_id: EventId,
    process: ProcessIdentity,
    sequence: u64,
    stream_id: Option<&str>,
) -> DomainEvent {
    let mut event = http_request_event(event_id, process);
    let EventPayload::Application(payload) = &mut event.payload else {
        unreachable!("http_request_event returns an application event");
    };
    payload
        .metadata
        .insert("payload_sequence".to_string(), sequence.to_string());
    if let Some(stream_id) = stream_id {
        payload
            .metadata
            .insert("stream_id".to_string(), stream_id.to_string());
    }
    event
}

fn http_response_event_with(
    event_id: EventId,
    process: ProcessIdentity,
    status_code: &str,
    reason: &str,
    sequence: u64,
    stream_id: Option<&str>,
) -> DomainEvent {
    let mut event = http_response_event(event_id, process);
    let EventPayload::Application(payload) = &mut event.payload else {
        unreachable!("http_response_event returns an application event");
    };
    payload.summary = format!("{status_code} {reason}");
    payload
        .metadata
        .insert("payload_sequence".to_string(), sequence.to_string());
    payload
        .metadata
        .insert("status_code".to_string(), status_code.to_string());
    payload
        .metadata
        .insert("reason".to_string(), reason.to_string());
    if let Some(stream_id) = stream_id {
        payload
            .metadata
            .insert("stream_id".to_string(), stream_id.to_string());
    }
    event
}
