// Tests for the corresponding production module.
use serde_json::json;

use super::structured_token_usage;

/// This protocol reaches the shared field mapping through its own entry
/// point. Testing only the `usage`-wrapper entry would leave exactly the
/// divergence that motivated sharing the table unguarded.
#[test]
fn reads_anthropic_names_through_the_event_entry_point() {
    let events = [json!({
        "input_tokens": 100,
        "output_tokens": 50,
        "cache_read_input_tokens": 800,
        "cache_creation_input_tokens": 200
    })];

    let usage = structured_token_usage(events.iter()).expect("usage from event object");

    assert_eq!(usage.prompt_tokens, Some(100));
    assert_eq!(usage.cached_prompt_tokens, Some(800));
    assert_eq!(usage.cache_creation_tokens, Some(200));
}

/// Counts live on the event object itself here, with no `usage` wrapper,
/// so a wrapped payload must not be mistaken for one.
#[test]
fn ignores_a_usage_wrapper_it_does_not_use() {
    let events = [json!({ "usage": { "prompt_tokens": 10 } })];
    assert!(structured_token_usage(events.iter()).is_none());
}

/// Every event is offered to the mapping and the last match wins, so a
/// later event carrying counts replaces an earlier one.
#[test]
fn the_last_event_carrying_counts_wins() {
    let events = [
        json!({ "prompt_tokens": 1, "completion_tokens": 2 }),
        json!({ "content": "no counts here" }),
        json!({ "prompt_tokens": 30, "completion_tokens": 40 }),
    ];

    let usage = structured_token_usage(events.iter()).expect("usage");

    assert_eq!(usage.prompt_tokens, Some(30));
    assert_eq!(usage.completion_tokens, Some(40));
}
