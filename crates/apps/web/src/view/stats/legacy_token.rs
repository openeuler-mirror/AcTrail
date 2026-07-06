//! Cross-trace stats JSON rendering for the web UI.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{SystemTime, UNIX_EPOCH};

use model_core::trace::TraceRecord;
use semantic_action::{SemanticAction, SemanticActionKind, SemanticActionLinkRole, attr_keys};
use storage_core::{StorageBackend, TraceFilter};

use crate::json;
use crate::view::traces;

const LLM_RESPONSE_KINDS: &[&str] = &[SemanticActionKind::LlmResponse.as_str()];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TokenUsageStatsQuery {
    pub from_ms: u64,
    pub to_ms: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct TokenCounts {
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
    total_tokens: Option<u64>,
    cached_prompt_tokens: Option<u64>,
    reasoning_tokens: Option<u64>,
    prompt_cache_hit_tokens: Option<u64>,
    prompt_cache_miss_tokens: Option<u64>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct TokenTotals {
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
    cached_prompt_tokens: u64,
    reasoning_tokens: u64,
    prompt_cache_hit_tokens: u64,
    prompt_cache_miss_tokens: u64,
}

struct TokenUsageRow {
    trace_id: u64,
    trace_name: String,
    response_action_id: String,
    request_action_id: Option<String>,
    started_at_ms: u64,
    model: Option<String>,
    provider_id: Option<String>,
    tokens: TokenCounts,
}

pub(crate) fn token_usage_stats_json(
    storage: &mut dyn StorageBackend,
    query: TokenUsageStatsQuery,
) -> Result<String, String> {
    let traces = storage
        .list_traces(&TraceFilter::default())
        .map_err(|error| {
            format!(
                "list traces for token stats failed: {}: {}",
                error.stage, error.message
            )
        })?;
    let trace_by_id = traces
        .iter()
        .map(|trace| (trace.trace_id.get(), trace))
        .collect::<BTreeMap<_, _>>();
    let mut rows = Vec::new();
    for trace in &traces {
        let mut trace_rows = token_usage_rows_for_trace(storage, trace, query)?;
        rows.append(&mut trace_rows);
    }
    rows.sort_by(|left, right| {
        left.started_at_ms
            .cmp(&right.started_at_ms)
            .then_with(|| left.trace_id.cmp(&right.trace_id))
            .then_with(|| left.response_action_id.cmp(&right.response_action_id))
    });
    Ok(token_usage_json(query, &trace_by_id, &rows))
}

fn token_usage_rows_for_trace(
    storage: &mut dyn StorageBackend,
    trace: &TraceRecord,
    query: TokenUsageStatsQuery,
) -> Result<Vec<TokenUsageRow>, String> {
    let trace_id = trace.trace_id;
    let responses = storage
        .semantic_actions_matching_kinds(trace_id, LLM_RESPONSE_KINDS)
        .map_err(|error| {
            format!(
                "list llm.response actions failed for trace {}: {}: {}",
                trace_id, error.stage, error.message
            )
        })?;
    if responses.is_empty() {
        return Ok(Vec::new());
    }
    let request_by_response = request_action_by_response(storage, trace_id.get())?;
    let mut rows = Vec::new();
    for response in responses {
        let started_at_ms = system_time_millis(response.start_time)
            .map_err(|error| format!("invalid start_time on {}: {error}", response.action_id))?;
        if started_at_ms < query.from_ms || started_at_ms >= query.to_ms {
            continue;
        }
        rows.push(token_usage_row(
            trace,
            &request_by_response,
            &response,
            started_at_ms,
        )?);
    }
    Ok(rows)
}

fn request_action_by_response(
    storage: &mut dyn StorageBackend,
    trace_id: u64,
) -> Result<BTreeMap<String, String>, String> {
    let links = storage
        .list_semantic_action_links(model_core::ids::TraceId::new(trace_id))
        .map_err(|error| {
            format!(
                "list semantic action links failed for trace {}: {}: {}",
                trace_id, error.stage, error.message
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

fn token_usage_row(
    trace: &TraceRecord,
    request_by_response: &BTreeMap<String, String>,
    response: &SemanticAction,
    started_at_ms: u64,
) -> Result<TokenUsageRow, String> {
    Ok(TokenUsageRow {
        trace_id: trace.trace_id.get(),
        trace_name: trace.display_name.as_str().to_string(),
        response_action_id: response.action_id.clone(),
        request_action_id: request_by_response.get(&response.action_id).cloned(),
        started_at_ms,
        model: non_empty_attribute(response, attr_keys::llm_response::MODEL),
        provider_id: non_empty_attribute(response, attr_keys::llm_response::PROVIDER_ID),
        tokens: token_counts(response)?,
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

fn token_counts(action: &SemanticAction) -> Result<TokenCounts, String> {
    Ok(TokenCounts {
        prompt_tokens: optional_u64_attr(action, attr_keys::llm_response::PROMPT_TOKENS)?,
        completion_tokens: optional_u64_attr(action, attr_keys::llm_response::COMPLETION_TOKENS)?,
        total_tokens: optional_u64_attr(action, attr_keys::llm_response::TOTAL_TOKENS)?,
        cached_prompt_tokens: optional_u64_attr(
            action,
            attr_keys::llm_response::CACHED_PROMPT_TOKENS,
        )?,
        reasoning_tokens: optional_u64_attr(action, attr_keys::llm_response::REASONING_TOKENS)?,
        prompt_cache_hit_tokens: optional_u64_attr(
            action,
            attr_keys::llm_response::PROMPT_CACHE_HIT_TOKENS,
        )?,
        prompt_cache_miss_tokens: optional_u64_attr(
            action,
            attr_keys::llm_response::PROMPT_CACHE_MISS_TOKENS,
        )?,
    })
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

fn token_usage_json(
    query: TokenUsageStatsQuery,
    trace_by_id: &BTreeMap<u64, &TraceRecord>,
    rows: &[TokenUsageRow],
) -> String {
    let request_rows = rows.iter().map(token_usage_row_json).collect::<Vec<_>>();
    let trace_rows = trace_by_id
        .values()
        .map(|trace| traces::trace_record_json(trace))
        .collect::<Vec<_>>();
    let mut output = String::from("{");
    json::field(&mut output, "range", &range_json(query));
    output.push(',');
    json::field(&mut output, "summary", &summary_json(rows));
    output.push(',');
    json::field(
        &mut output,
        "traces",
        &format!("[{}]", trace_rows.join(",")),
    );
    output.push(',');
    json::field(
        &mut output,
        "requests",
        &format!("[{}]", request_rows.join(",")),
    );
    output.push('}');
    output
}

fn range_json(query: TokenUsageStatsQuery) -> String {
    format!(
        "{{\"from_ms\":{},\"to_ms\":{}}}",
        json::number(query.from_ms),
        json::number(query.to_ms)
    )
}

fn summary_json(rows: &[TokenUsageRow]) -> String {
    let mut totals = TokenTotals::default();
    let mut trace_ids = BTreeSet::new();
    let mut models = BTreeSet::new();
    let mut usage_response_count = 0usize;
    for row in rows {
        trace_ids.insert(row.trace_id);
        if let Some(model) = row.model.as_ref() {
            models.insert(model.as_str());
        }
        if row.tokens.has_any() {
            usage_response_count += 1;
        }
        totals.add(row.tokens);
    }
    format!(
        "{{\"response_count\":{},\"usage_response_count\":{},\"missing_usage_count\":{},\"trace_count\":{},\"model_count\":{},\"prompt_tokens\":{},\"completion_tokens\":{},\"total_tokens\":{},\"cached_prompt_tokens\":{},\"reasoning_tokens\":{},\"prompt_cache_hit_tokens\":{},\"prompt_cache_miss_tokens\":{}}}",
        json::number(rows.len()),
        json::number(usage_response_count),
        json::number(rows.len().saturating_sub(usage_response_count)),
        json::number(trace_ids.len()),
        json::number(models.len()),
        json::number(totals.prompt_tokens),
        json::number(totals.completion_tokens),
        json::number(totals.total_tokens),
        json::number(totals.cached_prompt_tokens),
        json::number(totals.reasoning_tokens),
        json::number(totals.prompt_cache_hit_tokens),
        json::number(totals.prompt_cache_miss_tokens)
    )
}

fn token_usage_row_json(row: &TokenUsageRow) -> String {
    let mut output = String::from("{");
    json::field(&mut output, "trace_id", &json::number(row.trace_id));
    output.push(',');
    json::field(&mut output, "trace_name", &json::string(&row.trace_name));
    output.push(',');
    json::field(
        &mut output,
        "response_action_id",
        &json::string(&row.response_action_id),
    );
    output.push(',');
    json::field(
        &mut output,
        "request_action_id",
        &json::optional_string(row.request_action_id.as_deref()),
    );
    output.push(',');
    json::field(
        &mut output,
        "started_at_ms",
        &json::number(row.started_at_ms),
    );
    output.push(',');
    json::field(
        &mut output,
        "model",
        &json::optional_string(row.model.as_deref()),
    );
    output.push(',');
    json::field(
        &mut output,
        "provider_id",
        &json::optional_string(row.provider_id.as_deref()),
    );
    output.push(',');
    append_token_fields(&mut output, row.tokens);
    output.push('}');
    output
}

fn append_token_fields(output: &mut String, tokens: TokenCounts) {
    json::field(
        output,
        "prompt_tokens",
        &json::optional_number(tokens.prompt_tokens),
    );
    output.push(',');
    json::field(
        output,
        "completion_tokens",
        &json::optional_number(tokens.completion_tokens),
    );
    output.push(',');
    json::field(
        output,
        "total_tokens",
        &json::optional_number(tokens.total_tokens),
    );
    output.push(',');
    json::field(
        output,
        "cached_prompt_tokens",
        &json::optional_number(tokens.cached_prompt_tokens),
    );
    output.push(',');
    json::field(
        output,
        "reasoning_tokens",
        &json::optional_number(tokens.reasoning_tokens),
    );
    output.push(',');
    json::field(
        output,
        "prompt_cache_hit_tokens",
        &json::optional_number(tokens.prompt_cache_hit_tokens),
    );
    output.push(',');
    json::field(
        output,
        "prompt_cache_miss_tokens",
        &json::optional_number(tokens.prompt_cache_miss_tokens),
    );
}

fn system_time_millis(time: SystemTime) -> Result<u64, String> {
    let duration = time
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?;
    u64::try_from(duration.as_millis()).map_err(|error| error.to_string())
}

impl TokenCounts {
    fn has_any(self) -> bool {
        self.prompt_tokens
            .or(self.completion_tokens)
            .or(self.total_tokens)
            .or(self.cached_prompt_tokens)
            .or(self.reasoning_tokens)
            .or(self.prompt_cache_hit_tokens)
            .or(self.prompt_cache_miss_tokens)
            .is_some()
    }
}

impl TokenTotals {
    fn add(&mut self, counts: TokenCounts) {
        self.prompt_tokens += counts.prompt_tokens.unwrap_or(0);
        self.completion_tokens += counts.completion_tokens.unwrap_or(0);
        self.total_tokens += counts.total_tokens.unwrap_or(0);
        self.cached_prompt_tokens += counts.cached_prompt_tokens.unwrap_or(0);
        self.reasoning_tokens += counts.reasoning_tokens.unwrap_or(0);
        self.prompt_cache_hit_tokens += counts.prompt_cache_hit_tokens.unwrap_or(0);
        self.prompt_cache_miss_tokens += counts.prompt_cache_miss_tokens.unwrap_or(0);
    }
}
