use super::extract_token_usage;
use serde_json::json;

/// Anthropic Messages reports cache accounting as flat top-level keys.
/// The shared extractor only looked for the OpenAI nested shape, so these
/// were silently dropped for every request sent to /v1/messages.
#[test]
fn reads_anthropic_flat_cache_fields() {
    let usage = extract_token_usage(&json!({
        "usage": {
            "input_tokens": 100,
            "output_tokens": 50,
            "cache_read_input_tokens": 800,
            "cache_creation_input_tokens": 200
        }
    }))
    .expect("anthropic usage is recognised");

    assert_eq!(usage.prompt_tokens, Some(100));
    assert_eq!(usage.completion_tokens, Some(50));
    assert_eq!(usage.cached_prompt_tokens, Some(800));
    assert_eq!(usage.prompt_cache_hit_tokens, Some(800));
    // Cache creation is not a cache miss: it is prompt content the
    // provider stored for reuse, billed separately. Anthropic has no
    // field meaning "billed at full rate because the cache did not
    // serve it", so miss stays empty rather than borrowing this value.
    assert_eq!(usage.cache_creation_tokens, Some(200));
    assert_eq!(usage.prompt_cache_miss_tokens, None);
    // Anthropic does not report a total, so the parsed usage stays
    // faithful to the wire.
    assert_eq!(usage.total_tokens, None);
}

#[test]
fn reads_openai_nested_details() {
    let usage = extract_token_usage(&json!({
        "usage": {
            "prompt_tokens": 10,
            "completion_tokens": 20,
            "total_tokens": 30,
            "prompt_tokens_details": { "cached_tokens": 5 },
            "completion_tokens_details": { "reasoning_tokens": 7 }
        }
    }))
    .expect("openai usage is recognised");

    assert_eq!(usage.prompt_tokens, Some(10));
    assert_eq!(usage.completion_tokens, Some(20));
    assert_eq!(usage.total_tokens, Some(30));
    assert_eq!(usage.cached_prompt_tokens, Some(5));
    assert_eq!(usage.reasoning_tokens, Some(7));
}

#[test]
fn reads_openai_responses_input_output_details() {
    let usage = extract_token_usage(&json!({
        "usage": {
            "input_tokens": 11,
            "output_tokens": 22,
            "total_tokens": 33,
            "input_tokens_details": { "cached_tokens": 6 },
            "output_tokens_details": { "reasoning_tokens": 8 }
        }
    }))
    .expect("openai responses usage is recognised");

    assert_eq!(usage.prompt_tokens, Some(11));
    assert_eq!(usage.completion_tokens, Some(22));
    assert_eq!(usage.cached_prompt_tokens, Some(6));
    assert_eq!(usage.reasoning_tokens, Some(8));
}

/// DeepSeek's OpenAI-compatible endpoint reports cache accounting under its
/// own flat keys. This already worked and must keep working.
#[test]
fn reads_deepseek_flat_cache_fields() {
    let usage = extract_token_usage(&json!({
        "usage": {
            "prompt_tokens": 10,
            "completion_tokens": 20,
            "total_tokens": 30,
            "prompt_cache_hit_tokens": 8,
            "prompt_cache_miss_tokens": 2
        }
    }))
    .expect("deepseek usage is recognised");

    assert_eq!(usage.prompt_cache_hit_tokens, Some(8));
    assert_eq!(usage.prompt_cache_miss_tokens, Some(2));
    // DeepSeek reports a genuine miss count and never a creation count.
    assert_eq!(usage.cache_creation_tokens, None);
}

/// A nested OpenAI shape and a flat Anthropic shape must not fight: when
/// both appear the nested detail wins, because that is the shape the
/// provider that emits both actually documents.
#[test]
fn prefers_nested_details_when_both_shapes_present() {
    let usage = extract_token_usage(&json!({
        "usage": {
            "input_tokens": 1,
            "output_tokens": 2,
            "prompt_tokens_details": { "cached_tokens": 5 },
            "cache_read_input_tokens": 800
        }
    }))
    .expect("mixed usage is recognised");

    assert_eq!(usage.cached_prompt_tokens, Some(5));
}

#[test]
fn returns_none_when_no_counts_present() {
    assert!(extract_token_usage(&json!({ "usage": {} })).is_none());
    assert!(extract_token_usage(&json!({})).is_none());
}

/// Every precedence chain is pinned, not just the cached-prompt one: a
/// merge that quietly reorders any of them would otherwise pass.
#[test]
fn flat_keys_never_override_nested_details() {
    let usage = extract_token_usage(&json!({
        "usage": {
            "input_tokens": 1,
            "output_tokens": 2,
            "prompt_tokens_details": { "cached_tokens": 5 },
            "completion_tokens_details": { "reasoning_tokens": 9 },
            "cache_read_input_tokens": 800,
            "reasoning_tokens": 77
        }
    }))
    .expect("mixed usage is recognised");

    assert_eq!(usage.cached_prompt_tokens, Some(5));
    assert_eq!(usage.reasoning_tokens, Some(9));
}

/// DeepSeek's own hit count outranks Anthropic's cache read, which is the
/// same quantity under a name only Anthropic uses.
#[test]
fn a_reported_hit_count_outranks_a_cache_read() {
    let usage = extract_token_usage(&json!({
        "usage": {
            "prompt_tokens": 10,
            "prompt_cache_hit_tokens": 8,
            "cache_read_input_tokens": 800
        }
    }))
    .expect("usage is recognised");

    assert_eq!(usage.prompt_cache_hit_tokens, Some(8));
}

/// Anthropic's cache read is the only signal it gives for cached prompt
/// content, so it must reach both fields that describe that quantity.
#[test]
fn an_anthropic_cache_read_fills_both_cached_and_hit() {
    let usage = extract_token_usage(&json!({
        "usage": { "input_tokens": 1, "cache_read_input_tokens": 800 }
    }))
    .expect("usage is recognised");

    assert_eq!(usage.cached_prompt_tokens, Some(800));
    assert_eq!(usage.prompt_cache_hit_tokens, Some(800));
}
