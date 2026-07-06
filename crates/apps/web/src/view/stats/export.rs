use storage_core::StorageBackend;

use super::activity::{Dimension, activity_snapshot, top_dimension};
use super::model::{ExportView, LlmActivityQuery, LlmExportQuery};
use super::projector::project_llm_usage;

pub(crate) fn llm_export_csv(
    storage: &mut dyn StorageBackend,
    query: LlmExportQuery,
) -> Result<String, String> {
    let dataset = project_llm_usage(
        storage,
        LlmActivityQuery {
            from_ms: query.from_ms,
            to_ms: query.to_ms,
            rollup: None,
        },
    )?;
    Ok(match query.view {
        ExportView::Rows => rows_csv(&dataset.rows),
        ExportView::Overview => overview_csv(&activity_snapshot(&dataset)),
        ExportView::Explore => default_explore_csv(&dataset),
    })
}

fn rows_csv(rows: &[super::model::LlmUsageRow]) -> String {
    let mut output = csv_row([
        "started_at_ms",
        "trace_id",
        "trace_name",
        "response_action_id",
        "request_action_id",
        "model",
        "provider_id",
        "endpoint_label",
        "request_endpoint",
        "app_label",
        "app_executable",
        "has_usage",
        "total_tokens",
        "input_tokens",
        "output_tokens",
        "reasoning_tokens",
        "cache_hit_tokens",
        "cache_miss_tokens",
        "ttft_us",
        "tpot_us",
        "output_token_count",
    ]);
    for row in rows {
        output.push_str(&csv_row([
            row.started_at_ms.to_string(),
            row.trace_id.to_string(),
            row.trace_name.clone(),
            row.response_action_id.clone(),
            row.request_action_id.clone().unwrap_or_default(),
            row.model.clone().unwrap_or_default(),
            row.provider_id.clone().unwrap_or_default(),
            row.endpoint.label.clone().unwrap_or_default(),
            row.endpoint.canonical.clone().unwrap_or_default(),
            row.app.label.clone().unwrap_or_default(),
            row.app.executable.clone().unwrap_or_default(),
            row.tokens.has_any().to_string(),
            optional_number(row.tokens.total_tokens),
            optional_number(row.tokens.input_tokens),
            optional_number(row.tokens.output_tokens),
            optional_number(row.tokens.reasoning_tokens),
            optional_number(row.tokens.cache_hit_tokens),
            optional_number(row.tokens.cache_miss_tokens),
            optional_number(row.latency.ttft_us),
            optional_number(row.latency.tpot_us),
            optional_number(row.latency.output_token_count),
        ]));
    }
    output
}

fn overview_csv(snapshot: &super::model::ActivitySnapshot) -> String {
    let mut output = csv_row(["section", "key", "label", "value"]);
    output.push_str(&csv_row([
        "summary",
        "completed_requests",
        "Completed requests",
        snapshot.summary.completed_requests.to_string().as_str(),
    ]));
    output.push_str(&csv_row([
        "summary",
        "missing_usage_count",
        "Missing usage",
        snapshot.summary.missing_usage_count.to_string().as_str(),
    ]));
    output.push_str(&csv_row([
        "summary",
        "total_tokens",
        "Total tokens",
        snapshot.summary.totals.total_tokens.to_string().as_str(),
    ]));
    for (section, rows) in [
        ("top_models", snapshot.top_models.as_slice()),
        ("top_endpoints", snapshot.top_endpoints.as_slice()),
        ("top_apps", snapshot.top_apps.as_slice()),
        ("token_categories", snapshot.token_categories.as_slice()),
    ] {
        for row in rows {
            output.push_str(&csv_row([
                section,
                row.key.as_str(),
                row.label.as_str(),
                row.total.to_string().as_str(),
            ]));
        }
    }
    output
}

fn default_explore_csv(dataset: &super::model::LlmUsageDataset) -> String {
    let mut output = csv_row(["group", "label", "total_tokens", "completed_requests"]);
    for row in top_dimension(dataset, Dimension::Model, usize::MAX) {
        output.push_str(&csv_row([
            row.key.as_str(),
            row.label.as_str(),
            row.total.to_string().as_str(),
            row.completed_requests.to_string().as_str(),
        ]));
    }
    output
}

fn optional_number(value: Option<u64>) -> String {
    value.map(|value| value.to_string()).unwrap_or_default()
}

fn csv_row(values: impl IntoIterator<Item = impl AsRef<str>>) -> String {
    let values = values
        .into_iter()
        .map(|value| csv_cell(value.as_ref()))
        .collect::<Vec<_>>();
    format!("{}\n", values.join(","))
}

fn csv_cell(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}
