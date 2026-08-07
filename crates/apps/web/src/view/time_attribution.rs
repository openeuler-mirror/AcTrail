//! Wall-clock attribution for Agent-side, observable model-side, and unattributed time.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use model_core::ids::TraceId;
use model_core::process::{MembershipState, ProcessIdentity, ProcessMembership};
use model_core::trace::{TraceHealth, TraceLifecycleState, TraceRecord};
use semantic_action::{
    SemanticAction, SemanticActionCompleteness, SemanticActionKind, SemanticActionLink,
    SemanticActionLinkRole, SemanticActionStatus, attr_keys, validated_model_identifier,
};
use serde::Serialize;
use storage_core::{StorageBackend, TraceFilter};

#[path = "time_attribution/aggregate.rs"]
mod aggregate;
#[path = "time_attribution/cache.rs"]
mod cache;
#[path = "time_attribution/partition.rs"]
mod partition;
#[path = "time_attribution/projection.rs"]
mod projection;
#[path = "time_attribution/summary.rs"]
mod summary;
#[path = "time_attribution/turns.rs"]
mod turns;

use self::aggregate::{
    aggregate_breakdowns, aggregate_coverage, aggregate_issues, aggregate_status, attribution_row,
    dominant_category_target, project_range, sum_category_totals, trace_duration,
};
use self::cache::project_trace;
use self::summary::exact_percentages;

const SCHEMA_VERSION: &str = "time-attribution.v1";
const NANOS_PER_MILLI: u128 = 1_000_000;
const PERCENT_SCALE_BPS: u32 = 10_000;
const MODEL_UNKNOWN_KEY: &str = "__unknown_model__";
const MODEL_CONCURRENT_KEY: &str = "__concurrent_models__";
const TOOL_ORCHESTRATION_KEY: &str = "__orchestration__";
const TOOL_UNKNOWN_KEY: &str = "__unidentified_command__";
const TOOL_CONCURRENT_KEY: &str = "__concurrent_tools__";
const COMMAND_OVERHEAD_KEY: &str = "__tool_overhead__";
const COMMAND_CONCURRENT_KEY: &str = "__concurrent_commands__";
const BOTTLENECK_DEFAULT_DISPLAY_LIMIT: usize = 5;
const TERMINAL_CACHE_CAPACITY: usize = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TimeAttributionRangeQuery {
    pub from_ms: u64,
    pub to_ms: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TimeAttributionDimension {
    Category,
    Model,
    Tool,
    Command,
}

impl TimeAttributionDimension {
    pub(crate) fn parse(raw: &str) -> Result<Self, String> {
        match raw {
            "category" => Ok(Self::Category),
            "model" => Ok(Self::Model),
            "tool" => Ok(Self::Tool),
            "command" => Ok(Self::Command),
            _ => Err(format!(
                "unsupported time attribution dimension {raw}; expected category, model, tool, or command"
            )),
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Category => "category",
            Self::Model => "model",
            Self::Tool => "tool",
            Self::Command => "command",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TimeAttributionRowsQuery {
    pub range: TimeAttributionRangeQuery,
    pub offset: usize,
    pub limit: usize,
    pub dimension: Option<TimeAttributionDimension>,
    pub key: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
enum AttributionStatus {
    Complete,
    Provisional,
    Partial,
    Invalid,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
enum IssueSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Category {
    AgentSide,
    ModelSide,
    Unattributed,
}

impl Category {
    const ALL: [Self; 3] = [Self::AgentSide, Self::ModelSide, Self::Unattributed];

    const fn key(self) -> &'static str {
        match self {
            Self::AgentSide => "agent_side",
            Self::ModelSide => "model_side",
            Self::Unattributed => "unattributed",
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::AgentSide => "Agent-side",
            Self::ModelSide => "Model-side observable",
            Self::Unattributed => "Unattributed",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Interval {
    start: u128,
    end: u128,
}

impl Interval {
    fn new(start: u128, end: u128) -> Option<Self> {
        (start < end).then_some(Self { start, end })
    }

    fn duration(self) -> u128 {
        self.end - self.start
    }

    fn intersect(self, other: Self) -> Option<Self> {
        Self::new(self.start.max(other.start), self.end.min(other.end))
    }
}

#[derive(Clone, Debug)]
struct ModelInterval {
    interval: Interval,
    action_id: String,
    model: Option<String>,
    process: ProcessIdentity,
    status: &'static str,
    turn_key: UserTurnKey,
    user_input_start: Option<u128>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct UserTurnKey {
    process: ProcessIdentity,
    user_message_count: u64,
    latest_user_message_hash: String,
}

#[derive(Clone, Debug)]
struct UserTurn {
    interval: Interval,
    call_action_ids: Vec<String>,
}

#[derive(Clone, Debug)]
struct ToolInterval {
    interval: Interval,
    action_id: String,
    tool_name: Option<String>,
    process: ProcessIdentity,
}

#[derive(Clone, Debug)]
struct CommandInterval {
    interval: Interval,
    action_id: String,
    key: String,
    label: String,
    agent_tool_key: String,
    agent_tool_label: String,
    status: &'static str,
}

#[derive(Clone, Debug, Default)]
struct Boundary {
    agent_delta: i32,
    call_starts: Vec<usize>,
    call_ends: Vec<usize>,
    tool_starts: Vec<usize>,
    tool_ends: Vec<usize>,
}

#[derive(Clone, Debug, Serialize)]
struct AttributionIssue {
    code: String,
    severity: IssueSeverity,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    action_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    start_unix_nanos: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    end_unix_nanos: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
struct AttributionTarget {
    start_unix_nanos: String,
    end_unix_nanos: String,
    action_ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
struct DurationShare {
    key: String,
    label: String,
    duration_nanos: String,
    percentage_bps: u32,
    segment_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    target: Option<AttributionTarget>,
}

#[derive(Clone, Debug, Serialize)]
struct BreakdownShare {
    key: String,
    label: String,
    kind: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    agent_tools: Vec<String>,
    duration_nanos: String,
    percentage_bps: u32,
    segment_count: usize,
    action_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    target: Option<AttributionTarget>,
}

#[derive(Clone, Debug, Serialize)]
struct AttributionSegment {
    id: String,
    category: String,
    subcategory: String,
    key: String,
    label: String,
    start_unix_nanos: String,
    end_unix_nanos: String,
    duration_nanos: String,
    action_ids: Vec<String>,
    #[serde(skip)]
    interval: Interval,
    #[serde(skip)]
    category_value: Category,
}

#[derive(Clone, Debug, Serialize)]
struct CommandSegment {
    id: String,
    kind: String,
    key: String,
    label: String,
    agent_tools: Vec<String>,
    start_unix_nanos: String,
    end_unix_nanos: String,
    duration_nanos: String,
    action_ids: Vec<String>,
    #[serde(skip)]
    interval: Interval,
}

#[derive(Clone, Debug, Serialize)]
struct BottleneckInterval {
    id: String,
    kind: &'static str,
    key: String,
    label: String,
    description: String,
    status: &'static str,
    start_unix_nanos: String,
    end_unix_nanos: String,
    duration_nanos: String,
    action_ids: Vec<String>,
    #[serde(skip)]
    interval: Interval,
}

#[derive(Clone, Debug, Default, Serialize)]
struct BottleneckCollection {
    observed_count: usize,
    items: Vec<BottleneckInterval>,
}

#[derive(Clone, Debug, Serialize)]
struct TraceBottlenecks {
    default_display_limit: usize,
    model_requests: BottleneckCollection,
    commands: BottleneckCollection,
    unattributed_gaps: BottleneckCollection,
}

impl Default for TraceBottlenecks {
    fn default() -> Self {
        Self {
            default_display_limit: BOTTLENECK_DEFAULT_DISPLAY_LIMIT,
            model_requests: BottleneckCollection::default(),
            commands: BottleneckCollection::default(),
            unattributed_gaps: BottleneckCollection::default(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
struct RoundAttribution {
    id: String,
    label: String,
    description: String,
    kind: String,
    call_count: usize,
    start_unix_nanos: String,
    end_unix_nanos: String,
    duration_nanos: String,
    categories: Vec<DurationShare>,
    models: Vec<String>,
    tools: Vec<String>,
    action_ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
struct TraceReference {
    id: u64,
    name: String,
    state: String,
}

#[derive(Clone, Debug, Serialize)]
struct AttributionScope {
    start_unix_nanos: String,
    end_unix_nanos: String,
    duration_nanos: String,
    provisional: bool,
    semantics: &'static str,
    windows: Vec<AttributionScopeWindow>,
}

#[derive(Clone, Debug, Serialize)]
struct AttributionScopeWindow {
    id: String,
    start_unix_nanos: String,
    end_unix_nanos: String,
    duration_nanos: String,
}

#[derive(Clone, Debug, Default, Serialize)]
struct TraceCoverage {
    llm_request_count: usize,
    observed_llm_call_count: usize,
    llm_call_count: usize,
    excluded_llm_call_count: usize,
    user_turn_count: usize,
    strong_user_input_count: usize,
    agent_process_count: usize,
    tool_interval_count: usize,
    command_interval_count: usize,
    segment_count: usize,
    command_segment_count: usize,
}

#[derive(Clone, Debug, Serialize)]
struct TraceAttribution {
    schema_version: &'static str,
    trace: TraceReference,
    scope: AttributionScope,
    status: AttributionStatus,
    categories: Vec<DurationShare>,
    rounds: Vec<RoundAttribution>,
    models: Vec<BreakdownShare>,
    tools: Vec<BreakdownShare>,
    commands: Vec<BreakdownShare>,
    bottlenecks: TraceBottlenecks,
    segments: Vec<AttributionSegment>,
    command_segments: Vec<CommandSegment>,
    coverage: TraceCoverage,
    issues: Vec<AttributionIssue>,
}

#[derive(Clone, Debug, Serialize)]
struct AggregateRange {
    from_ms: u64,
    to_ms: u64,
    semantics: &'static str,
}

#[derive(Clone, Debug, Default, Serialize)]
struct AggregateCoverage {
    trace_count: usize,
    complete_trace_count: usize,
    provisional_trace_count: usize,
    partial_trace_count: usize,
    invalid_trace_count: usize,
    llm_request_count: usize,
    observed_llm_call_count: usize,
    llm_call_count: usize,
    excluded_llm_call_count: usize,
    user_turn_count: usize,
    tool_interval_count: usize,
    command_interval_count: usize,
}

#[derive(Clone, Debug, Serialize)]
struct AggregateIssue {
    code: String,
    severity: IssueSeverity,
    count: usize,
    message: String,
}

#[derive(Clone, Debug, Serialize)]
struct AggregateAttribution {
    schema_version: &'static str,
    range: AggregateRange,
    status: AttributionStatus,
    total_duration_nanos: String,
    categories: Vec<DurationShare>,
    models: Vec<BreakdownShare>,
    tools: Vec<BreakdownShare>,
    commands: Vec<BreakdownShare>,
    coverage: AggregateCoverage,
    issues: Vec<AggregateIssue>,
}

#[derive(Clone, Debug, Serialize)]
struct AttributionRow {
    trace: TraceReference,
    status: AttributionStatus,
    scope_duration_nanos: String,
    contribution_duration_nanos: String,
    percentage_bps: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    target: Option<AttributionTarget>,
}

#[derive(Clone, Debug, Serialize)]
struct Page {
    offset: usize,
    limit: usize,
    total: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_offset: Option<usize>,
}

#[derive(Clone, Debug, Serialize)]
struct RowsFilter {
    #[serde(skip_serializing_if = "Option::is_none")]
    dimension: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    key: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
struct AttributionRows {
    schema_version: &'static str,
    range: AggregateRange,
    filter: RowsFilter,
    page: Page,
    rows: Vec<AttributionRow>,
}

#[derive(Default)]
struct StatusTracker {
    partial: bool,
    invalid: bool,
    issues: Vec<AttributionIssue>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct FileRevision {
    modified_nanos: u128,
    len: u64,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct StorageRevision {
    database: Option<FileRevision>,
    write_ahead_log: Option<FileRevision>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct TerminalCacheKey {
    storage_path: String,
    trace_id: u64,
    terminal_nanos: u128,
    revision: StorageRevision,
}

struct TerminalProjectionCache {
    entries: HashMap<TerminalCacheKey, Arc<TraceAttribution>>,
    order: VecDeque<TerminalCacheKey>,
}

impl TerminalProjectionCache {
    fn new() -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    fn get(&mut self, key: &TerminalCacheKey) -> Option<Arc<TraceAttribution>> {
        let value = self.entries.get(key).cloned()?;
        self.order.retain(|candidate| candidate != key);
        self.order.push_back(key.clone());
        Some(value)
    }

    fn insert(&mut self, key: TerminalCacheKey, value: Arc<TraceAttribution>) {
        self.order.retain(|candidate| candidate != &key);
        while self.entries.len() >= TERMINAL_CACHE_CAPACITY {
            let Some(evicted) = self.order.pop_front() else {
                break;
            };
            self.entries.remove(&evicted);
        }
        self.order.push_back(key.clone());
        self.entries.insert(key, value);
    }

    fn clear(&mut self) -> usize {
        let count = self.entries.len();
        self.entries.clear();
        self.order.clear();
        count
    }
}

impl StatusTracker {
    fn info(&mut self, code: &str, message: impl Into<String>) {
        self.push(code, IssueSeverity::Info, message, None, None, None);
    }

    fn warning(&mut self, code: &str, message: impl Into<String>) {
        self.partial = true;
        self.push(code, IssueSeverity::Warning, message, None, None, None);
    }

    fn action_warning(
        &mut self,
        code: &str,
        message: impl Into<String>,
        action_id: &str,
        interval: Option<Interval>,
    ) {
        self.partial = true;
        self.push(
            code,
            IssueSeverity::Warning,
            message,
            Some(action_id.to_string()),
            interval.map(|value| value.start),
            interval.map(|value| value.end),
        );
    }

    fn action_info(
        &mut self,
        code: &str,
        message: impl Into<String>,
        action_id: &str,
        interval: Option<Interval>,
    ) {
        self.push(
            code,
            IssueSeverity::Info,
            message,
            Some(action_id.to_string()),
            interval.map(|value| value.start),
            interval.map(|value| value.end),
        );
    }

    fn action_error(&mut self, code: &str, message: impl Into<String>, action_id: &str) {
        self.partial = true;
        self.push(
            code,
            IssueSeverity::Error,
            message,
            Some(action_id.to_string()),
            None,
            None,
        );
    }

    fn invalid(&mut self, code: &str, message: impl Into<String>) {
        self.invalid = true;
        self.push(code, IssueSeverity::Error, message, None, None, None);
    }

    fn push(
        &mut self,
        code: &str,
        severity: IssueSeverity,
        message: impl Into<String>,
        action_id: Option<String>,
        start: Option<u128>,
        end: Option<u128>,
    ) {
        self.issues.push(AttributionIssue {
            code: code.to_string(),
            severity,
            message: message.into(),
            action_id,
            start_unix_nanos: start.map(nanos_string),
            end_unix_nanos: end.map(nanos_string),
        });
    }

    fn status(&self, provisional: bool) -> AttributionStatus {
        if self.invalid {
            AttributionStatus::Invalid
        } else if self.partial {
            AttributionStatus::Partial
        } else if provisional {
            AttributionStatus::Provisional
        } else {
            AttributionStatus::Complete
        }
    }
}

pub(super) fn trace_time_attribution_json(
    storage_path: &Path,
    storage: &mut dyn StorageBackend,
    trace_id: TraceId,
) -> Result<String, String> {
    let trace = storage
        .get_trace(trace_id)
        .map_err(|error| storage_error("read trace for time attribution", error))?
        .ok_or_else(|| format!("trace {trace_id} not found"))?;
    let attribution = project_trace(storage_path, storage, &trace, None)?;
    serde_json::to_string(&attribution)
        .map_err(|error| format!("serialize trace time attribution failed: {error}"))
}

pub(super) fn aggregate_time_attribution_json(
    storage_path: &Path,
    storage: &mut dyn StorageBackend,
    query: TimeAttributionRangeQuery,
) -> Result<String, String> {
    let window = range_interval(query)?;
    let projections = project_range(storage_path, storage, window)?;
    let total_duration = projections.iter().map(trace_duration).sum::<u128>();

    let category_totals = sum_category_totals(&projections);
    let category_percentages = exact_percentages(
        &Category::ALL.map(|category| {
            category_totals
                .get(category.key())
                .copied()
                .unwrap_or_default()
        }),
        total_duration,
    );
    let categories = Category::ALL
        .iter()
        .enumerate()
        .map(|(index, category)| {
            let duration = category_totals
                .get(category.key())
                .copied()
                .unwrap_or_default();
            DurationShare {
                key: category.key().to_string(),
                label: category.label().to_string(),
                duration_nanos: nanos_string(duration),
                percentage_bps: category_percentages[index],
                segment_count: projections
                    .iter()
                    .map(|projection| {
                        projection
                            .segments
                            .iter()
                            .filter(|segment| segment.category_value == *category)
                            .count()
                    })
                    .sum(),
                target: dominant_category_target(&projections, *category),
            }
        })
        .collect::<Vec<_>>();
    let models = aggregate_breakdowns(&projections, total_duration, |projection| {
        &projection.models
    });
    let tools = aggregate_breakdowns(&projections, total_duration, |projection| &projection.tools);
    let commands = aggregate_breakdowns(&projections, total_duration, |projection| {
        &projection.commands
    });
    let coverage = aggregate_coverage(&projections);
    let status = aggregate_status(&coverage);
    let issues = aggregate_issues(&projections);
    let response = AggregateAttribution {
        schema_version: SCHEMA_VERSION,
        range: AggregateRange {
            from_ms: query.from_ms,
            to_ms: query.to_ms,
            semantics: "trace_overlap_clipped",
        },
        status,
        total_duration_nanos: nanos_string(total_duration),
        categories,
        models,
        tools,
        commands,
        coverage,
        issues,
    };
    serde_json::to_string(&response)
        .map_err(|error| format!("serialize aggregate time attribution failed: {error}"))
}

pub(super) fn time_attribution_rows_json(
    storage_path: &Path,
    storage: &mut dyn StorageBackend,
    query: TimeAttributionRowsQuery,
) -> Result<String, String> {
    let window = range_interval(query.range)?;
    let projections = project_range(storage_path, storage, window)?;
    let mut rows = projections
        .iter()
        .filter_map(|projection| attribution_row(projection, &query))
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        parse_nanos(&right.contribution_duration_nanos)
            .cmp(&parse_nanos(&left.contribution_duration_nanos))
            .then_with(|| right.trace.id.cmp(&left.trace.id))
    });
    let total = rows.len();
    let page_rows = rows
        .into_iter()
        .skip(query.offset)
        .take(query.limit)
        .collect::<Vec<_>>();
    let next_offset = query
        .offset
        .checked_add(page_rows.len())
        .filter(|next| *next < total);
    let response = AttributionRows {
        schema_version: SCHEMA_VERSION,
        range: AggregateRange {
            from_ms: query.range.from_ms,
            to_ms: query.range.to_ms,
            semantics: "trace_overlap_clipped",
        },
        filter: RowsFilter {
            dimension: query.dimension.map(TimeAttributionDimension::as_str),
            key: query.key,
        },
        page: Page {
            offset: query.offset,
            limit: query.limit,
            total,
            next_offset,
        },
        rows: page_rows,
    };
    serde_json::to_string(&response)
        .map_err(|error| format!("serialize time attribution rows failed: {error}"))
}

pub(super) fn clear_time_attribution_cache() -> usize {
    cache::clear_time_attribution_cache()
}

fn range_interval(query: TimeAttributionRangeQuery) -> Result<Interval, String> {
    let start = u128::from(query.from_ms) * NANOS_PER_MILLI;
    let end = u128::from(query.to_ms) * NANOS_PER_MILLI;
    Interval::new(start, end)
        .ok_or_else(|| "time attribution range must have from_ms less than to_ms".to_string())
}

fn system_time_nanos(time: SystemTime) -> Result<u128, String> {
    time.duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .map_err(|error| format!("timestamp precedes Unix epoch: {error}"))
}

fn nanos_string(value: u128) -> String {
    value.to_string()
}

fn parse_nanos(value: &str) -> u128 {
    value.parse().unwrap_or_default()
}

fn non_empty_attr(action: &SemanticAction, key: &str) -> Option<String> {
    action
        .attributes
        .get(key)
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn valid_link(link: &SemanticActionLink) -> bool {
    link.valid
        && !link
            .attributes
            .get(attr_keys::actrail::LINK_VALID)
            .is_some_and(|value| value == "false")
}

fn storage_error(operation: &str, error: storage_core::StorageError) -> String {
    format!("{operation} failed: {}: {}", error.stage, error.message)
}
