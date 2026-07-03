use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LlmActivityQuery {
    pub from_ms: u64,
    pub to_ms: u64,
    pub rollup: Option<Rollup>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LlmRowsQuery {
    pub from_ms: u64,
    pub to_ms: u64,
    pub offset: usize,
    pub limit: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LlmExportQuery {
    pub from_ms: u64,
    pub to_ms: u64,
    pub view: ExportView,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ExportView {
    Overview,
    Explore,
    Rows,
}

impl ExportView {
    pub(crate) fn parse(raw: &str) -> Result<Self, String> {
        match raw {
            "overview" => Ok(Self::Overview),
            "explore" => Ok(Self::Explore),
            "rows" => Ok(Self::Rows),
            _ => Err(format!(
                "unsupported export view {raw}; expected overview, explore, or rows"
            )),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct LlmUsageDataset {
    pub range: LlmActivityQuery,
    pub rollup: Rollup,
    pub rows: Vec<LlmUsageRow>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct LlmUsageRow {
    pub trace_id: u64,
    pub trace_name: String,
    pub response_action_id: String,
    pub request_action_id: Option<String>,
    pub started_at_ms: u64,
    pub model: Option<String>,
    pub provider_id: Option<String>,
    pub endpoint: EndpointIdentity,
    pub app: AppIdentity,
    pub tokens: TokenUsage,
    pub latency: LlmLatency,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct EndpointIdentity {
    pub canonical: Option<String>,
    pub label: Option<String>,
    pub provider_fallback: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct AppIdentity {
    pub executable: Option<String>,
    pub label: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct TokenUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub cached_prompt_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub cache_hit_tokens: Option<u64>,
    pub cache_miss_tokens: Option<u64>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct LlmLatency {
    pub request_start_ms: Option<u64>,
    pub request_end_ms: Option<u64>,
    pub response_start_ms: u64,
    pub response_end_ms: Option<u64>,
    pub ttft_us: Option<u64>,
    pub tpot_us: Option<u64>,
    pub output_token_count: Option<u64>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct TokenTotals {
    pub total_tokens: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub cache_hit_tokens: u64,
    pub cache_miss_tokens: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct Coverage {
    pub usage_rows: usize,
    pub missing_usage_rows: usize,
    pub endpoint_missing_rows: usize,
    pub endpoint_provider_fallback_rows: usize,
    pub app_missing_rows: usize,
    pub priced_rows: usize,
    pub partially_priced_rows: usize,
    pub unpriced_rows: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ActivitySnapshot {
    pub range: LlmActivityQuery,
    pub rollup: Rollup,
    pub summary: Summary,
    pub coverage: Coverage,
    pub top_models: Vec<DimensionTotal>,
    pub top_endpoints: Vec<DimensionTotal>,
    pub top_apps: Vec<DimensionTotal>,
    pub token_categories: Vec<DimensionTotal>,
    pub model_trends: Vec<TrendSeries>,
    pub endpoint_trends: Vec<TrendSeries>,
    pub app_trends: Vec<TrendSeries>,
    pub token_category_trends: Vec<TrendSeries>,
    pub missing_usage_trend: Vec<BucketTotal>,
    pub latency: LatencySnapshot,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct Summary {
    pub completed_requests: usize,
    pub failed_requests: Option<usize>,
    pub missing_usage_count: usize,
    pub trace_count: usize,
    pub model_count: usize,
    pub endpoint_count: usize,
    pub app_count: usize,
    pub totals: TokenTotals,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct DimensionTotal {
    pub key: String,
    pub label: String,
    pub total: u64,
    pub completed_requests: usize,
    pub missing_usage_count: usize,
    pub trace_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct TrendSeries {
    pub key: String,
    pub label: String,
    pub total: u64,
    pub points: Vec<BucketTotal>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct BucketTotal {
    pub bucket_key: String,
    pub bucket_label: String,
    pub bucket_start_ms: u64,
    pub value: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct LatencySnapshot {
    pub ttft: LatencyDistribution,
    pub tpot: LatencyDistribution,
    pub grouped: LatencyGroupedSnapshot,
    pub trends: LatencyTrendSnapshot,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct LatencyGroupedSnapshot {
    pub models: Vec<LatencyGroupDistribution>,
    pub endpoints: Vec<LatencyGroupDistribution>,
    pub apps: Vec<LatencyGroupDistribution>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct LatencyGroupDistribution {
    pub key: String,
    pub label: String,
    pub ttft: LatencyDistribution,
    pub tpot: LatencyDistribution,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct LatencyTrendSnapshot {
    pub rollup: Rollup,
    pub models: Vec<LatencyTrendSeries>,
    pub endpoints: Vec<LatencyTrendSeries>,
    pub apps: Vec<LatencyTrendSeries>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct LatencyTrendSeries {
    pub key: String,
    pub label: String,
    pub ttft_avg: Vec<BucketTotal>,
    pub tpot_avg: Vec<BucketTotal>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct LatencyDistribution {
    pub sample_count: usize,
    pub missing_count: usize,
    pub min_us: Option<u64>,
    pub max_us: Option<u64>,
    pub mean_us: Option<u64>,
    pub p50_us: Option<u64>,
    pub p90_us: Option<u64>,
    pub p95_us: Option<u64>,
    pub p99_us: Option<u64>,
    pub samples_us: Vec<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Rollup {
    Minute,
    Hour,
    Day,
    Week,
    Month,
}

impl Rollup {
    pub(crate) fn parse(raw: &str) -> Result<Self, String> {
        match raw {
            "minute" => Ok(Self::Minute),
            "hour" => Ok(Self::Hour),
            "day" => Ok(Self::Day),
            "week" => Ok(Self::Week),
            "month" => Ok(Self::Month),
            _ => Err(format!(
                "unsupported rollup {raw}; expected minute, hour, day, week, or month"
            )),
        }
    }

    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::Minute => "minute",
            Self::Hour => "hour",
            Self::Day => "day",
            Self::Week => "week",
            Self::Month => "month",
        }
    }
}

impl TokenUsage {
    pub(super) fn has_any(self) -> bool {
        self.input_tokens
            .or(self.output_tokens)
            .or(self.total_tokens)
            .or(self.cached_prompt_tokens)
            .or(self.reasoning_tokens)
            .or(self.cache_hit_tokens)
            .or(self.cache_miss_tokens)
            .is_some()
    }
}

impl TokenTotals {
    pub(super) fn add_usage(&mut self, usage: TokenUsage) {
        self.total_tokens += usage.total_tokens.unwrap_or(0);
        self.input_tokens += usage.input_tokens.unwrap_or(0);
        self.output_tokens += usage.output_tokens.unwrap_or(0);
        self.reasoning_tokens += usage.reasoning_tokens.unwrap_or(0);
        self.cache_hit_tokens += usage.cache_hit_tokens.unwrap_or(0);
        self.cache_miss_tokens += usage.cache_miss_tokens.unwrap_or(0);
    }
}

pub(super) fn unique_count(values: impl Iterator<Item = Option<String>>) -> usize {
    values.flatten().collect::<BTreeSet<_>>().len()
}
