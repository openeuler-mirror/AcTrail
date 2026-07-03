use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use model_core::process::ProcessIdentity;
use model_core::trace::TraceRecord;
use semantic_action::{SemanticAction, SemanticActionKind, SemanticActionLinkRole, attr_keys};
use storage_core::{StorageBackend, TraceFilter};

use super::model::{
    AppIdentity, EndpointIdentity, LlmActivityQuery, LlmLatency, LlmUsageDataset, LlmUsageRow,
    Rollup, TokenUsage,
};
use super::time_buckets::bucket_start_ms;

const LLM_RESPONSE_KINDS: &[&str] = &[SemanticActionKind::LlmResponse.as_str()];
const CACHE_HIT_TOKEN_KEYS: &[&str] = &[
    attr_keys::llm_response::PROMPT_CACHE_HIT_TOKENS,
    attr_keys::llm_response::CACHED_PROMPT_TOKENS,
];
const CACHE_MISS_TOKEN_KEYS: &[&str] = &[attr_keys::llm_response::PROMPT_CACHE_MISS_TOKENS];

pub(super) fn project_llm_usage(
    storage: &mut dyn StorageBackend,
    query: LlmActivityQuery,
) -> Result<LlmUsageDataset, String> {
    let traces = storage
        .list_traces(&TraceFilter::default())
        .map_err(|error| {
            format!(
                "list traces for llm request stats failed: {}: {}",
                error.stage, error.message
            )
        })?;
    let mut rows = Vec::new();
    for trace in &traces {
        rows.append(&mut rows_for_trace(storage, trace, query)?);
    }
    rows.sort_by(|left, right| {
        left.started_at_ms
            .cmp(&right.started_at_ms)
            .then_with(|| left.trace_id.cmp(&right.trace_id))
            .then_with(|| left.response_action_id.cmp(&right.response_action_id))
    });
    let rollup = query
        .rollup
        .unwrap_or_else(|| default_rollup_for_rows(&rows, query.from_ms, query.to_ms));
    Ok(LlmUsageDataset {
        range: query,
        rollup,
        rows,
    })
}

fn rows_for_trace(
    storage: &mut dyn StorageBackend,
    trace: &TraceRecord,
    query: LlmActivityQuery,
) -> Result<Vec<LlmUsageRow>, String> {
    let responses = storage
        .semantic_actions_matching_kinds(trace.trace_id, LLM_RESPONSE_KINDS)
        .map_err(|error| {
            format!(
                "list llm.response actions failed for trace {}: {}: {}",
                trace.trace_id, error.stage, error.message
            )
        })?;
    if responses.is_empty() {
        return Ok(Vec::new());
    }
    let request_by_response = request_action_by_response(storage, trace.trace_id.get())?;
    let mut request_cache = BTreeMap::new();
    let mut app_cache = BTreeMap::new();
    let mut rows = Vec::new();
    for response in responses {
        let started_at_ms = system_time_millis(response.start_time)
            .map_err(|error| format!("invalid start_time on {}: {error}", response.action_id))?;
        if started_at_ms < query.from_ms || started_at_ms >= query.to_ms {
            continue;
        }
        rows.push(row_for_response(
            storage,
            trace,
            &request_by_response,
            &mut request_cache,
            &mut app_cache,
            &response,
            started_at_ms,
        )?);
    }
    Ok(rows)
}

fn row_for_response(
    storage: &mut dyn StorageBackend,
    trace: &TraceRecord,
    request_by_response: &BTreeMap<String, String>,
    request_cache: &mut BTreeMap<String, Option<SemanticAction>>,
    app_cache: &mut BTreeMap<ProcessIdentity, AppIdentity>,
    response: &SemanticAction,
    started_at_ms: u64,
) -> Result<LlmUsageRow, String> {
    let request_action_id = request_by_response.get(&response.action_id).cloned();
    let request = if let Some(action_id) = request_action_id.as_deref() {
        if !request_cache.contains_key(action_id) {
            let action = storage
                .semantic_action_by_id(trace.trace_id, action_id)
                .map_err(|error| {
                    format!(
                        "read linked llm.request action failed for trace {} action {}: {}: {}",
                        trace.trace_id, action_id, error.stage, error.message
                    )
                })?;
            request_cache.insert(action_id.to_string(), action);
        }
        request_cache.get(action_id).and_then(Option::as_ref)
    } else {
        None
    };
    let app = if let Some(identity) = app_cache.get(&response.process) {
        identity.clone()
    } else {
        let identity = app_identity(storage, trace, response)?;
        app_cache.insert(response.process.clone(), identity.clone());
        identity
    };
    let tokens = token_usage(response)?;
    let latency = latency_timing(request, response, tokens)?;
    Ok(LlmUsageRow {
        trace_id: trace.trace_id.get(),
        trace_name: trace.display_name.as_str().to_string(),
        response_action_id: response.action_id.clone(),
        request_action_id,
        started_at_ms,
        model: non_empty_attribute(response, attr_keys::llm_response::MODEL).or_else(|| {
            request.and_then(|action| non_empty_attribute(action, attr_keys::llm_request::MODEL))
        }),
        provider_id: non_empty_attribute(response, attr_keys::llm_response::PROVIDER_ID),
        endpoint: endpoint_identity(response, request),
        app,
        tokens,
        latency,
    })
}

fn request_action_by_response(
    storage: &mut dyn StorageBackend,
    trace_id: u64,
) -> Result<BTreeMap<String, String>, String> {
    let links = storage
        .list_semantic_action_links(model_core::ids::TraceId::new(trace_id))
        .map_err(|error| {
            format!(
                "list semantic action links failed for trace {trace_id}: {}: {}",
                error.stage, error.message
            )
        })?;
    let mut request_by_response = BTreeMap::new();
    let mut request_by_call = BTreeMap::new();
    let mut call_by_response = BTreeMap::new();
    for link in links {
        if !link.valid {
            continue;
        }
        match link.role {
            SemanticActionLinkRole::LlmRequestLlmResponse => {
                request_by_response.insert(link.child_action_id, link.parent_action_id);
            }
            SemanticActionLinkRole::LlmCallRequest => {
                request_by_call.insert(link.parent_action_id, link.child_action_id);
            }
            SemanticActionLinkRole::LlmCallResponse => {
                call_by_response.insert(link.child_action_id, link.parent_action_id);
            }
            _ => {}
        }
    }
    for (response_id, call_id) in call_by_response {
        if request_by_response.contains_key(&response_id) {
            continue;
        }
        if let Some(request_id) = request_by_call.get(&call_id) {
            request_by_response.insert(response_id, request_id.clone());
        }
    }
    Ok(request_by_response)
}

fn endpoint_identity(
    response: &SemanticAction,
    request: Option<&SemanticAction>,
) -> EndpointIdentity {
    let scheme = request
        .and_then(|action| non_empty_attribute(action, attr_keys::url::SCHEME))
        .or_else(|| non_empty_attribute(response, attr_keys::url::SCHEME));
    let address = request
        .and_then(|action| non_empty_attribute(action, attr_keys::server::ADDRESS))
        .or_else(|| non_empty_attribute(response, attr_keys::server::ADDRESS));
    let path = request
        .and_then(|action| non_empty_attribute(action, attr_keys::url::PATH))
        .or_else(|| non_empty_attribute(response, attr_keys::url::PATH));
    if let (Some(scheme), Some(address)) = (scheme, address) {
        let path = path.map(|value| strip_query(&value)).unwrap_or_default();
        let canonical = format!("{scheme}://{address}{path}");
        return EndpointIdentity {
            label: Some(endpoint_label(&canonical)),
            canonical: Some(canonical),
            provider_fallback: false,
        };
    }
    let provider = non_empty_attribute(response, attr_keys::llm_response::PROVIDER_ID);
    EndpointIdentity {
        label: provider.as_ref().map(|value| format!("[{value}]")),
        canonical: provider.map(|value| format!("provider:{value}")),
        provider_fallback: true,
    }
}

fn app_identity(
    storage: &mut dyn StorageBackend,
    trace: &TraceRecord,
    response: &SemanticAction,
) -> Result<AppIdentity, String> {
    let action = storage
        .semantic_action_for_process_kind(
            trace.trace_id,
            &response.process,
            SemanticActionKind::ProcessExec.as_str(),
        )
        .map_err(|error| {
            format!(
                "read process.exec action failed for trace {} response {}: {}: {}",
                trace.trace_id, response.action_id, error.stage, error.message
            )
        })?;
    let executable = action
        .as_ref()
        .and_then(|action| non_empty_attribute(action, attr_keys::process::EXECUTABLE));
    Ok(AppIdentity {
        label: executable.as_deref().map(basename),
        executable,
    })
}

fn token_usage(action: &SemanticAction) -> Result<TokenUsage, String> {
    let input_tokens = optional_u64_attr(action, attr_keys::llm_response::PROMPT_TOKENS)?;
    let output_tokens = optional_u64_attr(action, attr_keys::llm_response::COMPLETION_TOKENS)?;
    let reasoning_tokens = optional_u64_attr(action, attr_keys::llm_response::REASONING_TOKENS)?;
    let explicit_total = optional_u64_attr(action, attr_keys::llm_response::TOTAL_TOKENS)?;
    let cached_prompt_tokens =
        optional_u64_attr(action, attr_keys::llm_response::CACHED_PROMPT_TOKENS)?;
    let cache_hit_tokens = first_optional_u64_attr(action, CACHE_HIT_TOKEN_KEYS)?;
    let explicit_cache_miss_tokens = first_optional_u64_attr(action, CACHE_MISS_TOKEN_KEYS)?;
    let cache_miss_tokens = explicit_cache_miss_tokens.or_else(|| {
        input_tokens
            .zip(cache_hit_tokens)
            .and_then(|(input, hit)| input.checked_sub(hit))
    });
    let derived_total = derive_total(
        input_tokens,
        output_tokens,
        reasoning_tokens,
        cache_hit_tokens,
        cache_miss_tokens,
    );
    Ok(TokenUsage {
        input_tokens,
        output_tokens,
        total_tokens: explicit_total.or(derived_total),
        cached_prompt_tokens,
        reasoning_tokens,
        cache_hit_tokens,
        cache_miss_tokens,
    })
}

fn latency_timing(
    request: Option<&SemanticAction>,
    response: &SemanticAction,
    tokens: TokenUsage,
) -> Result<LlmLatency, String> {
    let request_start_ms = request
        .map(|action| system_time_millis(action.start_time))
        .transpose()?;
    let request_end_ms = request
        .and_then(|action| action.end_time)
        .map(system_time_millis)
        .transpose()?;
    let response_start_ms = system_time_millis(response.start_time)?;
    let response_end_ms = response.end_time.map(system_time_millis).transpose()?;
    let ttft_us = request.and_then(|action| {
        action
            .end_time
            .and_then(|end| duration_micros_between(action.start_time, end))
    });
    let output_token_count = output_token_count(tokens);
    let tpot_us = response
        .end_time
        .and_then(|end| duration_micros_between(response.start_time, end))
        .zip(output_token_count)
        .and_then(|(duration, token_count)| (token_count > 0).then_some(duration / token_count));
    Ok(LlmLatency {
        request_start_ms,
        request_end_ms,
        response_start_ms,
        response_end_ms,
        ttft_us,
        tpot_us,
        output_token_count,
    })
}

fn output_token_count(tokens: TokenUsage) -> Option<u64> {
    let output = tokens.output_tokens.unwrap_or(0);
    let reasoning = tokens.reasoning_tokens.unwrap_or(0);
    (tokens.output_tokens.or(tokens.reasoning_tokens).is_some()).then_some(output + reasoning)
}

fn duration_micros_between(start: SystemTime, end: SystemTime) -> Option<u64> {
    end.duration_since(start)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_micros()).ok())
}

fn derive_total(
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    reasoning_tokens: Option<u64>,
    cache_hit_tokens: Option<u64>,
    cache_miss_tokens: Option<u64>,
) -> Option<u64> {
    if input_tokens.or(output_tokens).is_some() {
        let reasoning_without_completion = output_tokens
            .is_none()
            .then_some(reasoning_tokens)
            .flatten();
        return Some(
            input_tokens.unwrap_or(0)
                + output_tokens.unwrap_or(0)
                + reasoning_without_completion.unwrap_or(0),
        );
    }
    if reasoning_tokens.is_some() {
        return reasoning_tokens;
    }
    cache_hit_tokens
        .or(cache_miss_tokens)
        .map(|_| cache_hit_tokens.unwrap_or(0) + cache_miss_tokens.unwrap_or(0))
}

fn first_optional_u64_attr(action: &SemanticAction, keys: &[&str]) -> Result<Option<u64>, String> {
    for key in keys {
        if action.attributes.contains_key(*key) {
            return optional_u64_attr(action, key);
        }
    }
    Ok(None)
}

fn optional_u64_attr(action: &SemanticAction, key: &str) -> Result<Option<u64>, String> {
    let Some(raw) = action.attributes.get(key) else {
        return Ok(None);
    };
    raw.parse::<u64>().map(Some).map_err(|error| {
        format!(
            "invalid token attribute {key} on {}: {error}",
            action.action_id
        )
    })
}

fn non_empty_attribute(action: &SemanticAction, key: &str) -> Option<String> {
    action
        .attributes
        .get(key)
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
}

fn endpoint_label(canonical: &str) -> String {
    let without_scheme = canonical
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(canonical);
    without_scheme
        .split('/')
        .next()
        .unwrap_or(without_scheme)
        .split(':')
        .next()
        .unwrap_or(without_scheme)
        .to_string()
}

fn strip_query(path: &str) -> String {
    path.split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path)
        .to_string()
}

fn basename(raw: &str) -> String {
    Path::new(raw)
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or(raw)
        .to_string()
}

fn default_rollup_for_rows(rows: &[LlmUsageRow], from_ms: u64, to_ms: u64) -> Rollup {
    const TARGET_MIN: usize = 8;
    const TARGET_MAX: usize = 20;
    const TARGET_IDEAL: isize = 14;
    const CANDIDATES: [Rollup; 5] = [
        Rollup::Minute,
        Rollup::Hour,
        Rollup::Day,
        Rollup::Week,
        Rollup::Month,
    ];

    if rows.is_empty() {
        return range_rollup(from_ms, to_ms);
    }

    let mut in_target = None::<(isize, usize, Rollup)>;
    let mut above_target = None::<(usize, Rollup)>;
    let mut below_target = None::<(usize, Rollup)>;
    for (index, rollup) in CANDIDATES.into_iter().enumerate() {
        let bucket_count = non_empty_bucket_count(rows, rollup);
        if bucket_count == 0 {
            continue;
        }
        if (TARGET_MIN..=TARGET_MAX).contains(&bucket_count) {
            let ideal_distance =
                (isize::try_from(bucket_count).unwrap_or(isize::MAX) - TARGET_IDEAL).abs();
            let score = (ideal_distance, index, rollup);
            if in_target
                .map(|current| (score.0, score.1) < (current.0, current.1))
                .unwrap_or(true)
            {
                in_target = Some(score);
            }
        } else if bucket_count > TARGET_MAX {
            if above_target
                .map(|current| bucket_count < current.0)
                .unwrap_or(true)
            {
                above_target = Some((bucket_count, rollup));
            }
        } else {
            if below_target
                .map(|current| bucket_count > current.0)
                .unwrap_or(true)
            {
                below_target = Some((bucket_count, rollup));
            }
        }
    }
    in_target
        .map(|(_, _, rollup)| rollup)
        .or_else(|| above_target.map(|(_, rollup)| rollup))
        .or_else(|| below_target.map(|(_, rollup)| rollup))
        .unwrap_or_else(|| range_rollup(from_ms, to_ms))
}

fn non_empty_bucket_count(rows: &[LlmUsageRow], rollup: Rollup) -> usize {
    rows.iter()
        .map(|row| bucket_start_ms(row.started_at_ms, rollup))
        .collect::<BTreeSet<_>>()
        .len()
}

fn range_rollup(from_ms: u64, to_ms: u64) -> Rollup {
    let range_ms = to_ms.saturating_sub(from_ms);
    if range_ms <= 6 * 60 * 60 * 1_000 {
        Rollup::Minute
    } else if range_ms <= 3 * 24 * 60 * 60 * 1_000 {
        Rollup::Hour
    } else if range_ms <= 120 * 24 * 60 * 60 * 1_000 {
        Rollup::Day
    } else if range_ms <= 2 * 365 * 24 * 60 * 60 * 1_000 {
        Rollup::Week
    } else {
        Rollup::Month
    }
}

fn system_time_millis(time: SystemTime) -> Result<u64, String> {
    let duration = time
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?;
    u64::try_from(duration.as_millis()).map_err(|error| error.to_string())
}
