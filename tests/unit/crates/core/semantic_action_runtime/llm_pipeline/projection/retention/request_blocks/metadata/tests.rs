// Tests for the corresponding production module.
use serde_json::json;

use super::{tool_result_count, user_message_metadata};

#[test]
fn counts_tool_results_across_supported_request_shapes() {
    let anthropic = json!({"messages": [{"role": "user", "content": [
        {"type": "tool_result", "tool_use_id": "a", "content": "one"},
        {"type": "tool-result", "tool_use_id": "b", "content": "two"}
    ]}]});
    let chat = json!({"messages": [
        {"role": "tool", "tool_call_id": "a", "content": "ok"}
    ]});
    let responses = json!({"input": [
        {"type": "function_call_output", "call_id": "a", "output": "ok"}
    ]});

    assert_eq!(tool_result_count(&anthropic), 2);
    assert_eq!(tool_result_count(&chat), 1);
    assert_eq!(tool_result_count(&responses), 1);
}

#[test]
fn tool_only_user_content_is_not_counted_as_user_input() {
    let body = json!({"messages": [{"role": "user", "content": [
        {"type": "tool_result", "tool_use_id": "a", "content": "ok"}
    ]}]});

    assert_eq!(tool_result_count(&body), 1);
    assert_eq!(user_message_metadata(&body).count, 0);
}
