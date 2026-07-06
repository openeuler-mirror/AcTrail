use crate::json;

use super::model::LlmUsageRow;

pub(super) fn row_json(row: &LlmUsageRow) -> String {
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
    json::field(
        &mut output,
        "endpoint_label",
        &json::optional_string(row.endpoint.label.as_deref()),
    );
    output.push(',');
    json::field(
        &mut output,
        "request_endpoint",
        &json::optional_string(row.endpoint.canonical.as_deref()),
    );
    output.push(',');
    json::field(
        &mut output,
        "endpoint_provider_fallback",
        &json::boolean(row.endpoint.provider_fallback),
    );
    output.push(',');
    json::field(
        &mut output,
        "app_label",
        &json::optional_string(row.app.label.as_deref()),
    );
    output.push(',');
    json::field(
        &mut output,
        "app_executable",
        &json::optional_string(row.app.executable.as_deref()),
    );
    output.push(',');
    json::field(
        &mut output,
        "has_usage",
        &json::boolean(row.tokens.has_any()),
    );
    output.push(',');
    json::field(
        &mut output,
        "input_tokens",
        &json::optional_number(row.tokens.input_tokens),
    );
    output.push(',');
    json::field(
        &mut output,
        "output_tokens",
        &json::optional_number(row.tokens.output_tokens),
    );
    output.push(',');
    json::field(
        &mut output,
        "total_tokens",
        &json::optional_number(row.tokens.total_tokens),
    );
    output.push(',');
    json::field(
        &mut output,
        "cached_prompt_tokens",
        &json::optional_number(row.tokens.cached_prompt_tokens),
    );
    output.push(',');
    json::field(
        &mut output,
        "reasoning_tokens",
        &json::optional_number(row.tokens.reasoning_tokens),
    );
    output.push(',');
    json::field(
        &mut output,
        "cache_hit_tokens",
        &json::optional_number(row.tokens.cache_hit_tokens),
    );
    output.push(',');
    json::field(
        &mut output,
        "cache_miss_tokens",
        &json::optional_number(row.tokens.cache_miss_tokens),
    );
    output.push(',');
    json::field(
        &mut output,
        "request_start_ms",
        &json::optional_number(row.latency.request_start_ms),
    );
    output.push(',');
    json::field(
        &mut output,
        "request_end_ms",
        &json::optional_number(row.latency.request_end_ms),
    );
    output.push(',');
    json::field(
        &mut output,
        "response_start_ms",
        &json::number(row.latency.response_start_ms),
    );
    output.push(',');
    json::field(
        &mut output,
        "response_end_ms",
        &json::optional_number(row.latency.response_end_ms),
    );
    output.push(',');
    json::field(
        &mut output,
        "ttft_us",
        &json::optional_number(row.latency.ttft_us),
    );
    output.push(',');
    json::field(
        &mut output,
        "tpot_us",
        &json::optional_number(row.latency.tpot_us),
    );
    output.push(',');
    json::field(
        &mut output,
        "output_token_count",
        &json::optional_number(row.latency.output_token_count),
    );
    output.push('}');
    output
}
