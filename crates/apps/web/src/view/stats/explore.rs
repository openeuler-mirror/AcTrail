use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;
use storage_core::StorageBackend;

use crate::json;

use super::activity::{Dimension, bucket_rows_json, dimension_key_label};
use super::model::{LlmActivityQuery, LlmUsageRow, Rollup};
use super::projector::project_llm_usage;
use super::time_buckets::{bucket_points, bucket_start_ms};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LlmExploreQuery {
    from_ms: u64,
    to_ms: u64,
    metric: Metric,
    group: Dimension,
    rollup: Rollup,
    top_n: usize,
    sort: ExploreSort,
    chart_kind: ChartKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Metric {
    TotalTokens,
    InputTokens,
    OutputTokens,
    ReasoningTokens,
    CacheHitTokens,
    CacheMissTokens,
    CompletedRequests,
    MissingUsageCount,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExploreSort {
    Top,
    Bottom,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ChartKind {
    Line,
    Histogram,
    Donut,
    Bar,
}

pub(crate) fn parse_explore_query(body: &str) -> Result<LlmExploreQuery, String> {
    let value = serde_json::from_str::<Value>(body)
        .map_err(|error| format!("invalid explore JSON body: {error}"))?;
    let from_ms = required_u64(&value, "from_ms")?;
    let to_ms = required_u64(&value, "to_ms")?;
    if from_ms >= to_ms {
        return Err("invalid explore range: from_ms must be less than to_ms".to_string());
    }
    let metric = Metric::parse(required_str(&value, "metric")?)?;
    let group = parse_group(required_str(&value, "group")?)?;
    let rollup = Rollup::parse(required_str(&value, "rollup")?)?;
    let top_n = required_usize(&value, "top_n")?;
    if top_n == 0 {
        return Err("invalid explore top_n: value must be positive".to_string());
    }
    let sort = ExploreSort::parse(required_str(&value, "sort")?)?;
    let chart_kind = ChartKind::parse(required_str(&value, "chart_kind")?)?;
    Ok(LlmExploreQuery {
        from_ms,
        to_ms,
        metric,
        group,
        rollup,
        top_n,
        sort,
        chart_kind,
    })
}

pub(crate) fn llm_explore_json(
    storage: &mut dyn StorageBackend,
    query: LlmExploreQuery,
) -> Result<String, String> {
    let dataset = project_llm_usage(
        storage,
        LlmActivityQuery {
            from_ms: query.from_ms,
            to_ms: query.to_ms,
            rollup: Some(query.rollup),
        },
    )?;
    let mut groups = BTreeMap::<String, ExploreGroup>::new();
    for row in &dataset.rows {
        let Some((key, label)) = dimension_key_label(row, query.group) else {
            continue;
        };
        groups
            .entry(key.clone())
            .or_insert_with(|| ExploreGroup {
                key,
                label,
                ..ExploreGroup::default()
            })
            .add(row, query.metric, query.rollup);
    }
    let total_value = groups.values().map(|group| group.total).sum::<u64>();
    let mut rows = groups.into_values().collect::<Vec<_>>();
    rows.sort_by(|left, right| match query.sort {
        ExploreSort::Top => right
            .total
            .cmp(&left.total)
            .then_with(|| left.label.cmp(&right.label)),
        ExploreSort::Bottom => left
            .total
            .cmp(&right.total)
            .then_with(|| left.label.cmp(&right.label)),
    });
    rows.truncate(query.top_n);
    Ok(explore_result_json(&query, total_value, &rows))
}

fn explore_result_json(query: &LlmExploreQuery, total_value: u64, rows: &[ExploreGroup]) -> String {
    let mut output = String::from("{");
    json::field(&mut output, "query", &query_json(query));
    output.push(',');
    json::field(&mut output, "total", &json::number(total_value));
    output.push(',');
    json::field(&mut output, "rows", &explore_rows_json(rows, total_value));
    output.push(',');
    json::field(&mut output, "series", &explore_series_json(rows));
    output.push('}');
    output
}

fn query_json(query: &LlmExploreQuery) -> String {
    format!(
        "{{\"from_ms\":{},\"to_ms\":{},\"metric\":{},\"group\":{},\"rollup\":{},\"top_n\":{},\"sort\":{},\"chart_kind\":{}}}",
        json::number(query.from_ms),
        json::number(query.to_ms),
        json::string(query.metric.as_str()),
        json::string(dimension_as_str(query.group)),
        json::string(query.rollup.as_str()),
        json::number(query.top_n),
        json::string(query.sort.as_str()),
        json::string(query.chart_kind.as_str())
    )
}

fn explore_rows_json(rows: &[ExploreGroup], total_value: u64) -> String {
    format!(
        "[{}]",
        rows.iter()
            .map(|row| {
                let share = if total_value == 0 {
                    0.0
                } else {
                    row.total as f64 / total_value as f64
                };
                format!(
                    "{{\"key\":{},\"label\":{},\"total\":{},\"share\":{},\"completed_requests\":{},\"missing_usage_count\":{},\"trace_count\":{}}}",
                    json::string(&row.key),
                    json::string(&row.label),
                    json::number(row.total),
                    json::number(share),
                    json::number(row.completed_requests),
                    json::number(row.missing_usage_count),
                    json::number(row.trace_ids.len())
                )
            })
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn explore_series_json(rows: &[ExploreGroup]) -> String {
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

#[derive(Default)]
struct ExploreGroup {
    key: String,
    label: String,
    total: u64,
    completed_requests: usize,
    missing_usage_count: usize,
    trace_ids: BTreeSet<u64>,
    points: Vec<super::model::BucketTotal>,
    point_values: BTreeMap<u64, u64>,
}

impl ExploreGroup {
    fn add(&mut self, row: &LlmUsageRow, metric: Metric, rollup: Rollup) {
        let value = metric.value(row);
        self.total += value;
        if row.tokens.has_any() {
            self.completed_requests += 1;
        } else {
            self.missing_usage_count += 1;
        }
        self.trace_ids.insert(row.trace_id);
        *self
            .point_values
            .entry(bucket_start_ms(row.started_at_ms, rollup))
            .or_default() += value;
        self.points = bucket_points(rollup, self.point_values.clone());
    }
}

impl Metric {
    fn parse(raw: &str) -> Result<Self, String> {
        match raw {
            "total_tokens" => Ok(Self::TotalTokens),
            "input_tokens" => Ok(Self::InputTokens),
            "output_tokens" => Ok(Self::OutputTokens),
            "reasoning_tokens" => Ok(Self::ReasoningTokens),
            "cache_hit_tokens" => Ok(Self::CacheHitTokens),
            "cache_miss_tokens" => Ok(Self::CacheMissTokens),
            "completed_requests" => Ok(Self::CompletedRequests),
            "missing_usage_count" => Ok(Self::MissingUsageCount),
            "estimated_spend_cny" => Err(
                "metric estimated_spend_cny is unavailable because pricing is disabled in v1"
                    .to_string(),
            ),
            _ => Err(format!("unsupported explore metric {raw}")),
        }
    }

    fn value(self, row: &LlmUsageRow) -> u64 {
        match self {
            Self::TotalTokens => row.tokens.total_tokens.unwrap_or(0),
            Self::InputTokens => row.tokens.input_tokens.unwrap_or(0),
            Self::OutputTokens => row.tokens.output_tokens.unwrap_or(0),
            Self::ReasoningTokens => row.tokens.reasoning_tokens.unwrap_or(0),
            Self::CacheHitTokens => row.tokens.cache_hit_tokens.unwrap_or(0),
            Self::CacheMissTokens => row.tokens.cache_miss_tokens.unwrap_or(0),
            Self::CompletedRequests => {
                if row.tokens.has_any() {
                    1
                } else {
                    0
                }
            }
            Self::MissingUsageCount => {
                if row.tokens.has_any() {
                    0
                } else {
                    1
                }
            }
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::TotalTokens => "total_tokens",
            Self::InputTokens => "input_tokens",
            Self::OutputTokens => "output_tokens",
            Self::ReasoningTokens => "reasoning_tokens",
            Self::CacheHitTokens => "cache_hit_tokens",
            Self::CacheMissTokens => "cache_miss_tokens",
            Self::CompletedRequests => "completed_requests",
            Self::MissingUsageCount => "missing_usage_count",
        }
    }
}

impl ExploreSort {
    fn parse(raw: &str) -> Result<Self, String> {
        match raw {
            "top" => Ok(Self::Top),
            "bottom" => Ok(Self::Bottom),
            _ => Err(format!(
                "unsupported explore sort {raw}; expected top or bottom"
            )),
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Top => "top",
            Self::Bottom => "bottom",
        }
    }
}

impl ChartKind {
    fn parse(raw: &str) -> Result<Self, String> {
        match raw {
            "line" => Ok(Self::Line),
            "histogram" => Ok(Self::Histogram),
            "donut" => Ok(Self::Donut),
            "bar" => Ok(Self::Bar),
            _ => Err(format!(
                "unsupported explore chart_kind {raw}; expected line, histogram, donut, or bar"
            )),
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Line => "line",
            Self::Histogram => "histogram",
            Self::Donut => "donut",
            Self::Bar => "bar",
        }
    }
}

fn parse_group(raw: &str) -> Result<Dimension, String> {
    match raw {
        "model" => Ok(Dimension::Model),
        "endpoint" => Ok(Dimension::Endpoint),
        "app" => Ok(Dimension::App),
        _ => Err(format!(
            "unsupported explore group {raw}; expected model, endpoint, or app"
        )),
    }
}

fn dimension_as_str(dimension: Dimension) -> &'static str {
    match dimension {
        Dimension::Model => "model",
        Dimension::Endpoint => "endpoint",
        Dimension::App => "app",
    }
}

fn required_str<'a>(value: &'a Value, key: &'static str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing or invalid explore field {key}"))
}

fn required_u64(value: &Value, key: &'static str) -> Result<u64, String> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("missing or invalid explore field {key}"))
}

fn required_usize(value: &Value, key: &'static str) -> Result<usize, String> {
    let raw = required_u64(value, key)?;
    usize::try_from(raw).map_err(|error| format!("invalid explore field {key}: {error}"))
}
