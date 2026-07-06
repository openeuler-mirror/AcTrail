use std::collections::{BTreeMap, BTreeSet};

use storage_core::StorageBackend;

use crate::json;

use super::model::{
    ActivitySnapshot, BucketTotal, Coverage, DimensionTotal, LlmActivityQuery, LlmRowsQuery,
    LlmUsageDataset, LlmUsageRow, Rollup, Summary, TokenTotals, TrendSeries, unique_count,
};
use super::projector::project_llm_usage;
use super::render::row_json;
use super::time_buckets::{bucket_points, bucket_start_ms};

#[path = "activity/latency.rs"]
mod latency;
use self::latency::{latency_snapshot, latency_snapshot_json};

const TOP_DIMENSION_LIMIT: usize = 8;

pub(crate) fn llm_activity_json(
    storage: &mut dyn StorageBackend,
    query: LlmActivityQuery,
) -> Result<String, String> {
    let dataset = project_llm_usage(storage, query)?;
    Ok(activity_snapshot_json(&activity_snapshot(&dataset)))
}

pub(crate) fn llm_request_rows_json(
    storage: &mut dyn StorageBackend,
    query: LlmRowsQuery,
) -> Result<String, String> {
    let dataset = project_llm_usage(
        storage,
        LlmActivityQuery {
            from_ms: query.from_ms,
            to_ms: query.to_ms,
            rollup: None,
        },
    )?;
    let total = dataset.rows.len();
    let rows = dataset
        .rows
        .iter()
        .rev()
        .skip(query.offset)
        .take(query.limit)
        .map(row_json)
        .collect::<Vec<_>>();
    let mut output = String::from("{");
    json::field(
        &mut output,
        "range",
        &range_json(dataset.range, dataset.rollup),
    );
    output.push(',');
    json::field(
        &mut output,
        "page",
        &format!(
            "{{\"offset\":{},\"limit\":{},\"total\":{},\"has_more\":{}}}",
            json::number(query.offset),
            json::number(query.limit),
            json::number(total),
            json::boolean(query.offset.saturating_add(query.limit) < total)
        ),
    );
    output.push(',');
    json::field(&mut output, "rows", &format!("[{}]", rows.join(",")));
    output.push('}');
    Ok(output)
}

pub(super) fn activity_snapshot(dataset: &LlmUsageDataset) -> ActivitySnapshot {
    ActivitySnapshot {
        range: dataset.range,
        rollup: dataset.rollup,
        summary: summary(dataset),
        coverage: coverage(dataset),
        top_models: top_dimension(dataset, Dimension::Model, TOP_DIMENSION_LIMIT),
        top_endpoints: top_dimension(dataset, Dimension::Endpoint, TOP_DIMENSION_LIMIT),
        top_apps: top_dimension(dataset, Dimension::App, TOP_DIMENSION_LIMIT),
        token_categories: token_categories(dataset),
        model_trends: trend_series(dataset, Dimension::Model, usize::MAX),
        endpoint_trends: trend_series(dataset, Dimension::Endpoint, usize::MAX),
        app_trends: trend_series(dataset, Dimension::App, usize::MAX),
        token_category_trends: token_category_trends(dataset),
        missing_usage_trend: missing_usage_trend(dataset),
        latency: latency_snapshot(dataset),
    }
}

fn summary(dataset: &LlmUsageDataset) -> Summary {
    let mut totals = TokenTotals::default();
    let mut trace_ids = BTreeSet::new();
    let mut completed_requests = 0usize;
    let mut missing_usage_count = 0usize;
    for row in &dataset.rows {
        trace_ids.insert(row.trace_id);
        if row.tokens.has_any() {
            completed_requests += 1;
            totals.add_usage(row.tokens);
        } else {
            missing_usage_count += 1;
        }
    }
    Summary {
        completed_requests,
        failed_requests: None,
        missing_usage_count,
        trace_count: trace_ids.len(),
        model_count: unique_count(dataset.rows.iter().map(|row| row.model.clone())),
        endpoint_count: unique_count(
            dataset
                .rows
                .iter()
                .map(|row| row.endpoint.canonical.clone()),
        ),
        app_count: unique_count(dataset.rows.iter().map(|row| row.app.executable.clone())),
        totals,
    }
}

fn coverage(dataset: &LlmUsageDataset) -> Coverage {
    let mut coverage = Coverage::default();
    for row in &dataset.rows {
        if row.tokens.has_any() {
            coverage.usage_rows += 1;
        } else {
            coverage.missing_usage_rows += 1;
        }
        if row.endpoint.canonical.is_none() {
            coverage.endpoint_missing_rows += 1;
        }
        if row.endpoint.provider_fallback {
            coverage.endpoint_provider_fallback_rows += 1;
        }
        if row.app.executable.is_none() {
            coverage.app_missing_rows += 1;
        }
    }
    coverage.unpriced_rows = coverage.usage_rows;
    coverage
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Dimension {
    Model,
    Endpoint,
    App,
}

pub(super) fn top_dimension(
    dataset: &LlmUsageDataset,
    dimension: Dimension,
    limit: usize,
) -> Vec<DimensionTotal> {
    let mut groups = BTreeMap::<String, DimensionAccumulator>::new();
    for row in dataset.rows.iter().filter(|row| row.tokens.has_any()) {
        let Some((key, label)) = dimension_key_label(row, dimension) else {
            continue;
        };
        groups
            .entry(key.clone())
            .or_insert_with(|| DimensionAccumulator {
                key,
                label,
                ..DimensionAccumulator::default()
            })
            .add(row);
    }
    let mut rows = groups
        .into_values()
        .map(DimensionAccumulator::finish)
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        right
            .total
            .cmp(&left.total)
            .then_with(|| right.completed_requests.cmp(&left.completed_requests))
            .then_with(|| left.label.cmp(&right.label))
    });
    rows.truncate(limit);
    rows
}

fn trend_series(dataset: &LlmUsageDataset, dimension: Dimension, limit: usize) -> Vec<TrendSeries> {
    let selected = top_dimension(dataset, dimension, limit)
        .into_iter()
        .map(|row| (row.key, row.label))
        .collect::<BTreeMap<_, _>>();
    let mut points = BTreeMap::<String, BTreeMap<u64, u64>>::new();
    for row in &dataset.rows {
        let Some((key, _)) = dimension_key_label(row, dimension) else {
            continue;
        };
        if !selected.contains_key(&key) {
            continue;
        }
        let bucket = bucket_start_ms(row.started_at_ms, dataset.rollup);
        *points.entry(key).or_default().entry(bucket).or_default() +=
            row.tokens.total_tokens.unwrap_or(0);
    }
    selected
        .into_iter()
        .map(|(key, label)| {
            let values = points.remove(&key).unwrap_or_default();
            let total = values.values().sum();
            TrendSeries {
                key,
                label,
                total,
                points: bucket_points(dataset.rollup, values),
            }
        })
        .collect()
}

fn token_categories(dataset: &LlmUsageDataset) -> Vec<DimensionTotal> {
    token_category_defs()
        .into_iter()
        .filter_map(|definition| {
            let total = dataset.rows.iter().map(|row| (definition.value)(row)).sum();
            (total > 0).then(|| DimensionTotal {
                key: definition.key.to_string(),
                label: definition.label.to_string(),
                total,
                completed_requests: dataset
                    .rows
                    .iter()
                    .filter(|row| row.tokens.has_any())
                    .count(),
                missing_usage_count: dataset
                    .rows
                    .iter()
                    .filter(|row| !row.tokens.has_any())
                    .count(),
                trace_count: dataset
                    .rows
                    .iter()
                    .map(|row| row.trace_id)
                    .collect::<BTreeSet<_>>()
                    .len(),
            })
        })
        .collect()
}

fn token_category_trends(dataset: &LlmUsageDataset) -> Vec<TrendSeries> {
    token_category_defs()
        .into_iter()
        .filter_map(|definition| {
            let mut values = BTreeMap::<u64, u64>::new();
            for row in &dataset.rows {
                *values
                    .entry(bucket_start_ms(row.started_at_ms, dataset.rollup))
                    .or_default() += (definition.value)(row);
            }
            let total = values.values().sum();
            (total > 0).then(|| TrendSeries {
                key: definition.key.to_string(),
                label: definition.label.to_string(),
                total,
                points: bucket_points(dataset.rollup, values),
            })
        })
        .collect()
}

fn missing_usage_trend(dataset: &LlmUsageDataset) -> Vec<BucketTotal> {
    let mut values = BTreeMap::<u64, u64>::new();
    for row in dataset.rows.iter().filter(|row| !row.tokens.has_any()) {
        *values
            .entry(bucket_start_ms(row.started_at_ms, dataset.rollup))
            .or_default() += 1;
    }
    bucket_points(dataset.rollup, values)
}

fn activity_snapshot_json(snapshot: &ActivitySnapshot) -> String {
    let mut output = String::from("{");
    json::field(
        &mut output,
        "range",
        &range_json(snapshot.range, snapshot.rollup),
    );
    output.push(',');
    json::field(&mut output, "capabilities", &capabilities_json());
    output.push(',');
    json::field(&mut output, "summary", &summary_json(&snapshot.summary));
    output.push(',');
    json::field(&mut output, "coverage", &coverage_json(&snapshot.coverage));
    output.push(',');
    json::field(&mut output, "overview", &overview_json(snapshot));
    output.push(',');
    json::field(&mut output, "trends", &trends_json(snapshot));
    output.push(',');
    json::field(
        &mut output,
        "latency",
        &latency_snapshot_json(&snapshot.latency),
    );
    output.push('}');
    output
}

pub(super) fn range_json(query: LlmActivityQuery, rollup: Rollup) -> String {
    format!(
        "{{\"from_ms\":{},\"to_ms\":{},\"rollup\":{}}}",
        json::number(query.from_ms),
        json::number(query.to_ms),
        json::string(rollup.as_str())
    )
}

fn capabilities_json() -> String {
    "{\"pricing\":false,\"failed_requests\":false,\"guardrails\":false}".to_string()
}

fn summary_json(summary: &Summary) -> String {
    format!(
        "{{\"completed_requests\":{},\"failed_requests\":{},\"missing_usage_count\":{},\"trace_count\":{},\"model_count\":{},\"endpoint_count\":{},\"app_count\":{},\"total_tokens\":{},\"input_tokens\":{},\"output_tokens\":{},\"reasoning_tokens\":{},\"cache_hit_tokens\":{},\"cache_miss_tokens\":{},\"estimated_spend_cny\":null}}",
        json::number(summary.completed_requests),
        json::optional_number(summary.failed_requests),
        json::number(summary.missing_usage_count),
        json::number(summary.trace_count),
        json::number(summary.model_count),
        json::number(summary.endpoint_count),
        json::number(summary.app_count),
        json::number(summary.totals.total_tokens),
        json::number(summary.totals.input_tokens),
        json::number(summary.totals.output_tokens),
        json::number(summary.totals.reasoning_tokens),
        json::number(summary.totals.cache_hit_tokens),
        json::number(summary.totals.cache_miss_tokens)
    )
}

fn coverage_json(coverage: &Coverage) -> String {
    format!(
        "{{\"usage_rows\":{},\"missing_usage_rows\":{},\"endpoint_missing_rows\":{},\"endpoint_provider_fallback_rows\":{},\"app_missing_rows\":{},\"priced_rows\":{},\"partially_priced_rows\":{},\"unpriced_rows\":{}}}",
        json::number(coverage.usage_rows),
        json::number(coverage.missing_usage_rows),
        json::number(coverage.endpoint_missing_rows),
        json::number(coverage.endpoint_provider_fallback_rows),
        json::number(coverage.app_missing_rows),
        json::number(coverage.priced_rows),
        json::number(coverage.partially_priced_rows),
        json::number(coverage.unpriced_rows)
    )
}

fn overview_json(snapshot: &ActivitySnapshot) -> String {
    format!(
        "{{\"top_models\":{},\"top_endpoints\":{},\"top_apps\":{},\"token_categories\":{},\"charts\":{{\"token_categories\":{}}}}}",
        dimension_rows_json(&snapshot.top_models),
        dimension_rows_json(&snapshot.top_endpoints),
        dimension_rows_json(&snapshot.top_apps),
        dimension_rows_json(&snapshot.token_categories),
        dimension_rows_json(&snapshot.token_categories)
    )
}

fn trends_json(snapshot: &ActivitySnapshot) -> String {
    format!(
        "{{\"rollup\":{},\"models\":{},\"endpoints\":{},\"apps\":{},\"token_categories\":{},\"missing_usage\":{}}}",
        json::string(snapshot.rollup.as_str()),
        trend_rows_json(&snapshot.model_trends),
        trend_rows_json(&snapshot.endpoint_trends),
        trend_rows_json(&snapshot.app_trends),
        trend_rows_json(&snapshot.token_category_trends),
        bucket_rows_json(&snapshot.missing_usage_trend)
    )
}

pub(super) fn dimension_rows_json(rows: &[DimensionTotal]) -> String {
    format!(
        "[{}]",
        rows.iter()
            .map(|row| {
                format!(
                    "{{\"key\":{},\"label\":{},\"total\":{},\"completed_requests\":{},\"missing_usage_count\":{},\"trace_count\":{}}}",
                    json::string(&row.key),
                    json::string(&row.label),
                    json::number(row.total),
                    json::number(row.completed_requests),
                    json::number(row.missing_usage_count),
                    json::number(row.trace_count)
                )
            })
            .collect::<Vec<_>>()
            .join(",")
    )
}

pub(super) fn trend_rows_json(rows: &[TrendSeries]) -> String {
    format!(
        "[{}]",
        rows.iter()
            .map(|row| {
                format!(
                    "{{\"key\":{},\"label\":{},\"total\":{},\"points\":{}}}",
                    json::string(&row.key),
                    json::string(&row.label),
                    json::number(row.total),
                    bucket_rows_json(&row.points)
                )
            })
            .collect::<Vec<_>>()
            .join(",")
    )
}

pub(super) fn bucket_rows_json(rows: &[BucketTotal]) -> String {
    format!(
        "[{}]",
        rows.iter()
            .map(|row| {
                format!(
                    "{{\"bucket_key\":{},\"bucket_label\":{},\"bucket_start_ms\":{},\"value\":{}}}",
                    json::string(&row.bucket_key),
                    json::string(&row.bucket_label),
                    json::number(row.bucket_start_ms),
                    json::number(row.value)
                )
            })
            .collect::<Vec<_>>()
            .join(",")
    )
}

#[derive(Default)]
struct DimensionAccumulator {
    key: String,
    label: String,
    total: u64,
    completed_requests: usize,
    missing_usage_count: usize,
    trace_ids: BTreeSet<u64>,
}

impl DimensionAccumulator {
    fn add(&mut self, row: &LlmUsageRow) {
        self.total += row.tokens.total_tokens.unwrap_or(0);
        if row.tokens.has_any() {
            self.completed_requests += 1;
        } else {
            self.missing_usage_count += 1;
        }
        self.trace_ids.insert(row.trace_id);
    }

    fn finish(self) -> DimensionTotal {
        DimensionTotal {
            key: self.key,
            label: self.label,
            total: self.total,
            completed_requests: self.completed_requests,
            missing_usage_count: self.missing_usage_count,
            trace_count: self.trace_ids.len(),
        }
    }
}

struct TokenCategoryDefinition {
    key: &'static str,
    label: &'static str,
    value: fn(&LlmUsageRow) -> u64,
}

fn token_category_defs() -> Vec<TokenCategoryDefinition> {
    vec![
        TokenCategoryDefinition {
            key: "input",
            label: "Input",
            value: |row| row.tokens.input_tokens.unwrap_or(0),
        },
        TokenCategoryDefinition {
            key: "output",
            label: "Output",
            value: |row| {
                row.tokens
                    .output_tokens
                    .unwrap_or(0)
                    .saturating_sub(row.tokens.reasoning_tokens.unwrap_or(0))
            },
        },
        TokenCategoryDefinition {
            key: "reasoning",
            label: "Reasoning",
            value: |row| row.tokens.reasoning_tokens.unwrap_or(0),
        },
        TokenCategoryDefinition {
            key: "cache_hit",
            label: "Cache Hit",
            value: |row| row.tokens.cache_hit_tokens.unwrap_or(0),
        },
        TokenCategoryDefinition {
            key: "cache_miss",
            label: "Cache Miss",
            value: |row| row.tokens.cache_miss_tokens.unwrap_or(0),
        },
    ]
}

pub(super) fn dimension_key_label(
    row: &LlmUsageRow,
    dimension: Dimension,
) -> Option<(String, String)> {
    match dimension {
        Dimension::Model => row
            .model
            .clone()
            .or_else(|| Some("(unknown model)".to_string()))
            .map(|value| (value.clone(), value)),
        Dimension::Endpoint => row
            .endpoint
            .canonical
            .clone()
            .zip(row.endpoint.label.clone())
            .or_else(|| {
                Some((
                    "(unknown endpoint)".to_string(),
                    "(unknown endpoint)".to_string(),
                ))
            }),
        Dimension::App => row
            .app
            .executable
            .clone()
            .zip(row.app.label.clone())
            .or_else(|| Some(("(unknown app)".to_string(), "(unknown app)".to_string()))),
    }
}
