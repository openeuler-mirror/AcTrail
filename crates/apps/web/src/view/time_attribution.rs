//! Wall-clock attribution for Agent-side, observable model-side, and unattributed time.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
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

#[path = "time_attribution/agents.rs"]
mod agents;
#[path = "time_attribution/aggregate.rs"]
mod aggregate;
#[path = "time_attribution/api.rs"]
mod api;
#[path = "time_attribution/cache.rs"]
mod cache;
#[path = "time_attribution/model.rs"]
mod model;
#[path = "time_attribution/partition.rs"]
mod partition;
#[path = "time_attribution/projection.rs"]
mod projection;
#[path = "time_attribution/summary.rs"]
mod summary;
#[path = "time_attribution/turns.rs"]
mod turns;

use self::aggregate::{
    aggregate_breakdowns, aggregate_coverage, aggregate_issues, aggregate_status,
    aggregate_tool_workloads, attribution_row, dominant_category_target, project_range,
    sum_category_totals, trace_duration,
};
pub(crate) use self::api::{
    TimeAttributionDimension, TimeAttributionRangeQuery, TimeAttributionRowsQuery,
};
pub(super) use self::api::{
    aggregate_time_attribution_json, clear_time_attribution_cache, time_attribution_rows_json,
    trace_time_attribution_json,
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
    agent_invocation: bool,
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
struct ToolWorkload {
    key: String,
    label: String,
    call_count: usize,
    measured_interval_count: usize,
    measured_duration_nanos: Option<String>,
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
    #[serde(skip_serializing_if = "Vec::is_empty")]
    agent_tools: Vec<String>,
    #[serde(skip)]
    interval: Interval,
    #[serde(skip)]
    category_value: Category,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum CommandSegmentKind {
    ToolOverhead,
    ConcurrentCommands,
    Command,
}

impl CommandSegmentKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::ToolOverhead => "tool_overhead",
            Self::ConcurrentCommands => "concurrent_commands",
            Self::Command => "command",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
struct CommandSegment {
    id: String,
    kind: CommandSegmentKind,
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

impl CommandSegment {
    const fn contains_command_interval(&self) -> bool {
        !matches!(self.kind, CommandSegmentKind::ToolOverhead)
    }

    fn unique_command_count(segments: &[Self]) -> usize {
        segments
            .iter()
            .filter(|segment| segment.contains_command_interval())
            .flat_map(|segment| segment.action_ids.iter().map(String::as_str))
            .collect::<BTreeSet<_>>()
            .len()
    }
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
    #[serde(skip)]
    workload_tool_intervals: Vec<ToolInterval>,
    #[serde(skip)]
    tool_calls: Vec<model::ToolCallOccurrence>,
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
    tool_workloads: Vec<ToolWorkload>,
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
}

fn storage_error(operation: &str, error: storage_core::StorageError) -> String {
    format!("{operation} failed: {}: {}", error.stage, error.message)
}
