#![cfg_attr(not(test), no_std)]
#![cfg_attr(test, allow(dead_code))]

extern crate alloc;

use alloc::collections::{BTreeMap, BTreeSet, VecDeque};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::fmt::Write;

use serde::{Deserialize, Serialize};
use spin::Mutex;

wit_bindgen::generate!({
    path: "../../../../crates/core/plugin_system/wit",
    world: "observation-plugin",
});

use actrail::plugin::types::{
    AlertDraft, AlertWriteRequest, ConfigReadStatus, SemanticActionRecord,
};
use exports::actrail::plugin::observation_consumer::{Guest, ObservationBatch, ObservationReport};

// ============================================================================
// 常量
// ============================================================================

const ALERT_KEY_FREQUENT_FAILURE: &str = "frequent-failure";
const ALERT_KEY_INDETERMINATE: &str = "indeterminate-result";

const CONFIG_CHUNK_BYTES: u64 = 4096;
const CONFIG_MAX_BYTES: usize = 16384;

const ATTR_PROCESS_ID: &str = "process.id";
const ATTR_PROCESS_PARENT_ID: &str = "process.parent.id";
const ATTR_PROCESS_EXIT_CODE: &str = "process.exit_code";
const ATTR_PROCESS_FAILURE_SUMMARY: &str = "process.failure.summary";
const ATTR_COMMAND_TOOL_NAME: &str = "command.tool.name";
const ATTR_COMMAND_LINE: &str = "command.line";
const ATTR_PROCESS_EXECUTABLE: &str = "process.executable";
const ATTR_MCP_TOOL_NAME: &str = "mcp.tool.name";
const ATTR_MCP_EXECUTION_STATUS: &str = "mcp.execution.status";
const ATTR_LLM_TOOL_CALLS_JSON: &str = "llm.response.tool_calls_json";
const ATTR_ENFORCEMENT_OPERATION: &str = "enforcement.operation";
const ATTR_ENFORCEMENT_DECISION: &str = "enforcement.decision";
const ATTR_ENFORCEMENT_RESULT: &str = "enforcement.result";

const RAW_EXIT_CODE_NONZERO: &str = "exit_code_nonzero";
const RAW_COMMAND_ERROR: &str = "command_error";
const RAW_MCP_ERROR: &str = "mcp_error";
const RAW_POLICY_DENIED: &str = "policy_denied";

// ============================================================================
// 配置
// ============================================================================

#[derive(Clone, Debug, Deserialize)]
#[serde(default, rename_all = "snake_case")]
struct PluginConfig {
    alert: AlertConfig,
    filter: FilterConfig,
    failure_type_map: BTreeMap<String, String>,
    strict_mapping: bool,
    evidence: EvidenceConfig,
    desensitization: DesensitizationConfig,
    debug_include_command_line: bool,
    reporting: ReportingConfig,
    resources: ResourcesConfig,
}

impl Default for PluginConfig {
    fn default() -> Self {
        Self {
            alert: AlertConfig::default(),
            filter: FilterConfig::default(),
            failure_type_map: BTreeMap::from([
                (
                    RAW_EXIT_CODE_NONZERO.to_string(),
                    "runtime_error".to_string(),
                ),
                (RAW_COMMAND_ERROR.to_string(), "runtime_error".to_string()),
                (RAW_MCP_ERROR.to_string(), "mcp_error".to_string()),
                (RAW_POLICY_DENIED.to_string(), "policy_denied".to_string()),
                ("indeterminate".to_string(), "unknown".to_string()),
            ]),
            strict_mapping: false,
            evidence: EvidenceConfig { max_count: 64 },
            desensitization: DesensitizationConfig::default(),
            debug_include_command_line: false,
            reporting: ReportingConfig {
                mode: "database".to_string(),
                endpoint: String::new(),
                enabled: false,
            },
            resources: ResourcesConfig::default(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, rename_all = "snake_case")]
struct AlertConfig {
    enabled: bool,
    trigger_mode: TriggerMode,
    min_failure_count: u64,
    min_failure_rate: f64,
    window_seconds: u64,
    cooldown_seconds: u64,
    after_alert: AfterAlertMode,
    indeterminate_handling: IndeterminateHandling,
    unknown_counts_as_success: bool,
}

impl Default for AlertConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            trigger_mode: TriggerMode::Count,
            min_failure_count: 3,
            min_failure_rate: 0.0,
            window_seconds: 60,
            cooldown_seconds: 60,
            after_alert: AfterAlertMode::ResetWindow,
            indeterminate_handling: IndeterminateHandling::Skip,
            unknown_counts_as_success: true,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, rename_all = "snake_case")]
struct FilterConfig {
    tool_scope: ToolScope,
    parent_scope: ParentScope,
    llm_attribution: LlmAttribution,
    monitored_tools: Vec<String>,
    ignored_tools: Vec<String>,
}

impl Default for FilterConfig {
    fn default() -> Self {
        Self {
            tool_scope: ToolScope::LlmAndMcp,
            parent_scope: ParentScope::AgentChild,
            llm_attribution: LlmAttribution::Fifo,
            monitored_tools: Vec::new(),
            ignored_tools: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, rename_all = "snake_case")]
struct EvidenceConfig {
    max_count: usize,
}

impl Default for EvidenceConfig {
    fn default() -> Self {
        Self { max_count: 64 }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, rename_all = "snake_case")]
struct DesensitizationConfig {
    mode: DesensitizationMode,
    summary_max_chars: usize,
    redact_keywords: Vec<String>,
}

impl Default for DesensitizationConfig {
    fn default() -> Self {
        Self {
            mode: DesensitizationMode::CategoryOnly,
            summary_max_chars: 120,
            redact_keywords: vec![
                "sk-".to_string(),
                "api_key".to_string(),
                "apikey".to_string(),
                "password".to_string(),
                "token".to_string(),
                "Authorization".to_string(),
                "secret".to_string(),
            ],
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, rename_all = "snake_case")]
struct ReportingConfig {
    mode: String,
    endpoint: String,
    enabled: bool,
}

impl Default for ReportingConfig {
    fn default() -> Self {
        Self {
            mode: "database".to_string(),
            endpoint: String::new(),
            enabled: false,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, rename_all = "snake_case")]
struct ResourcesConfig {
    state_ttl_seconds: u64,
    pending_queue_capacity: usize,
    max_trace_states: usize,
    attribution_grace_seconds: u64,
}

impl Default for ResourcesConfig {
    fn default() -> Self {
        Self {
            state_ttl_seconds: 600,
            pending_queue_capacity: 1024,
            max_trace_states: 1024,
            attribution_grace_seconds: 10,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum TriggerMode {
    Count,
    Rate,
    CountAndRate,
    CountOrRate,
}

impl Default for TriggerMode {
    fn default() -> Self {
        Self::Count
    }
}

impl TriggerMode {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Count => "count",
            Self::Rate => "rate",
            Self::CountAndRate => "count_and_rate",
            Self::CountOrRate => "count_or_rate",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum AfterAlertMode {
    ResetWindow,
    KeepWindow,
}

impl Default for AfterAlertMode {
    fn default() -> Self {
        Self::ResetWindow
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum IndeterminateHandling {
    Skip,
    Diagnostic,
}

impl Default for IndeterminateHandling {
    fn default() -> Self {
        Self::Skip
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum ToolScope {
    LlmAndMcp,
    McpOnly,
    AgentChildren,
}

impl Default for ToolScope {
    fn default() -> Self {
        Self::LlmAndMcp
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum ParentScope {
    AgentChild,
    Any,
}

impl Default for ParentScope {
    fn default() -> Self {
        Self::AgentChild
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum LlmAttribution {
    Fifo,
    SkipIfAmbiguous,
}

impl Default for LlmAttribution {
    fn default() -> Self {
        Self::Fifo
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum DesensitizationMode {
    CategoryOnly,
    Sanitized,
    Raw,
}

impl Default for DesensitizationMode {
    fn default() -> Self {
        Self::CategoryOnly
    }
}

impl PluginConfig {
    fn load() -> Result<Self, String> {
        let mut bytes = Vec::new();
        let mut offset = 0_u64;
        loop {
            let chunk = actrail::plugin::host::read_config(offset, CONFIG_CHUNK_BYTES);
            match chunk.status {
                ConfigReadStatus::Ok => {}
                ConfigReadStatus::NotConfigured => return Ok(Self::default()),
                ConfigReadStatus::TooLarge => {
                    return Err("tool-frequent-failure-alert config exceeds host limit".to_string());
                }
            }
            bytes.extend_from_slice(&chunk.bytes);
            if bytes.len() > CONFIG_MAX_BYTES {
                return Err("tool-frequent-failure-alert config exceeds 16384 bytes".to_string());
            }
            let Some(next_offset) = chunk.next_offset else {
                break;
            };
            if next_offset <= offset {
                return Err(
                    "tool-frequent-failure-alert config next offset did not advance".to_string(),
                );
            }
            offset = next_offset;
        }
        let raw = core::str::from_utf8(&bytes)
            .map_err(|error| format!("tool-frequent-failure-alert config is not UTF-8: {error}"))?;
        let config = serde_json::from_str::<Self>(raw)
            .map_err(|error| format!("parse tool-frequent-failure-alert config failed: {error}"))?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), String> {
        if self.reporting.mode != "database" {
            return Err(format!(
                "reporting.mode must be \"database\", got \"{}\"",
                self.reporting.mode
            ));
        }
        if self.alert.window_seconds == 0 {
            return Err("alert.window_seconds must be greater than zero".into());
        }
        if self.alert.min_failure_count == 0 {
            return Err("alert.min_failure_count must be greater than zero".into());
        }
        if !(0.0..=1.0).contains(&self.alert.min_failure_rate) {
            return Err("alert.min_failure_rate must be between 0.0 and 1.0".into());
        }
        if self.evidence.max_count == 0 || self.evidence.max_count > 1024 {
            return Err("evidence.max_count must be between 1 and 1024".into());
        }
        if self.resources.pending_queue_capacity == 0
            || self.resources.pending_queue_capacity > 8192
        {
            return Err("resources.pending_queue_capacity must be between 1 and 8192".into());
        }
        if self.resources.max_trace_states == 0 || self.resources.max_trace_states > 4096 {
            return Err("resources.max_trace_states must be between 1 and 4096".into());
        }
        if self.resources.attribution_grace_seconds > 300 {
            return Err("resources.attribution_grace_seconds must be between 0 and 300".into());
        }
        Ok(())
    }

    fn should_monitor(&self, tool_name: &str) -> bool {
        if self
            .filter
            .ignored_tools
            .iter()
            .any(|pattern| pattern_matches(tool_name, pattern))
        {
            return false;
        }
        if self.filter.monitored_tools.is_empty() {
            return true;
        }
        self.filter
            .monitored_tools
            .iter()
            .any(|pattern| pattern_matches(tool_name, pattern))
    }

    fn map_failure_type(&self, raw: &str) -> String {
        if let Some(mapped) = self.failure_type_map.get(raw) {
            return mapped.clone();
        }
        if self.strict_mapping {
            "other".to_string()
        } else {
            raw.to_string()
        }
    }
}

fn pattern_matches(name: &str, pattern: &str) -> bool {
    if let Some(prefix) = pattern.strip_suffix('*') {
        name.starts_with(prefix)
    } else {
        name == pattern
    }
}

// ============================================================================
// 状态机
// ============================================================================

/// 聚合维度：按 (trace, 工具名) 聚合。
/// 失败类型与退出状态作为窗口内分布字段展示，不参与聚合键，
/// 否则同一工具不同退出码的失败会被拆散（真实 Agent 场景无法触发告警）。
type WindowKey = String;

#[derive(Clone)]
struct CommandEntry {
    action_id: String,
    tool_name: String,
    process_id: String,
    command_line: String,
    match_command_lines: Vec<String>,
    executable: String,
    observed_ms: u64,
    monitored: bool,
    /// 工具名是否来自 LLM 归因/宿主回填（权威）：为 true 时进程的 exec 替换不再覆盖，
    /// 避免 opencode 的 bash 工具被 exec 成 ls 后丢失工具名。
    llm_attributed: bool,
    deferred_attribution: bool,
}

struct WindowState {
    window_start_ms: u64,
    failure_count: u64,
    success_count: u64,
    /// (失败类型, 退出状态) → 次数，用于告警展示主导类别与分布
    breakdown: BTreeMap<(String, String), u64>,
    first_action_id: String,
    last_action_id: String,
    /// 已处理 action_id（成败都记录），用于跨 batch 重发去重
    dedup_ids: VecDeque<String>,
    /// 失败证据 action_id 列表（上限 evidence.max_count）
    failure_evidence: Vec<String>,
    failure_evidence_truncated: u64,
    last_alert_ms: u64,
    last_active_ms: u64,
}

impl WindowState {
    fn new(now_ms: u64) -> Self {
        Self {
            window_start_ms: now_ms,
            failure_count: 0,
            success_count: 0,
            breakdown: BTreeMap::new(),
            first_action_id: String::new(),
            last_action_id: String::new(),
            dedup_ids: VecDeque::new(),
            failure_evidence: Vec::new(),
            failure_evidence_truncated: 0,
            last_alert_ms: 0,
            last_active_ms: now_ms,
        }
    }

    fn touch(&mut self, now_ms: u64, window_ms: u64) {
        self.last_active_ms = now_ms;
        if now_ms.saturating_sub(self.window_start_ms) >= window_ms {
            self.reset(now_ms);
        }
    }

    fn reset(&mut self, now_ms: u64) {
        // 保留冷却时间：窗口重置不能清空 last_alert_ms
        let last_alert = self.last_alert_ms;
        *self = Self::new(now_ms);
        self.last_alert_ms = last_alert;
    }

    fn seen(&self, action_id: &str) -> bool {
        self.dedup_ids.iter().any(|id| id == action_id)
    }

    fn push_evidence(&mut self, action_ids: &[&str], dedup_cap: usize, evidence_max: usize) {
        for action_id in action_ids {
            if self.dedup_ids.len() >= dedup_cap {
                self.dedup_ids.pop_front();
            }
            self.dedup_ids.push_back(action_id.to_string());
            if self.failure_evidence.len() >= evidence_max {
                self.failure_evidence_truncated = self.failure_evidence_truncated.saturating_add(1);
            } else {
                self.failure_evidence.push(action_id.to_string());
            }
        }
    }

    fn should_alert(&self, cfg: &PluginConfig, now_ms: u64) -> bool {
        if !cfg.alert.enabled {
            return false;
        }
        if self.failure_count < cfg.alert.min_failure_count {
            return false;
        }
        let total = self.failure_count + self.success_count;
        let rate = if total > 0 {
            self.failure_count as f64 / total as f64
        } else {
            0.0
        };
        let rate_ok = rate >= cfg.alert.min_failure_rate;
        let pass = match cfg.alert.trigger_mode {
            TriggerMode::Count => true,
            TriggerMode::Rate => rate_ok,
            TriggerMode::CountAndRate => rate_ok,
            TriggerMode::CountOrRate => true,
        };
        if !pass {
            return false;
        }
        if cfg.alert.cooldown_seconds > 0
            && self.last_alert_ms > 0
            && now_ms.saturating_sub(self.last_alert_ms)
                < cfg.alert.cooldown_seconds.saturating_mul(1000)
        {
            return false;
        }
        true
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Outcome {
    Success,
    Failure,
    Indeterminate,
}

/// 一次待匹配的 LLM 工具调用。
/// `hint` 取自工具参数（如 opencode bash 工具的 arguments.command），
/// 用于和后续 command.invocation 的命令行做边界匹配，避免被无关进程抢先消耗。
struct PendingToolCall {
    name: String,
    hint: String,
    observed_ms: u64,
}

struct DeferredExecution {
    result: ProcessExitResult,
    exit_action_id: String,
    observed_ms: u64,
}

struct TraceState {
    agent_process_id: Option<String>,
    pending_tool_calls: VecDeque<PendingToolCall>,
    /// 已观察到的 LLM tool call 标识及首次观察时间。流式响应可能反复投递
    /// 同一个调用；不去重会留下过期提示，阻塞后续顺序工具调用的延迟归因。
    seen_tool_calls: BTreeMap<String, u64>,
    /// 正在执行 LLM 工具调用的进程 id：其子进程视为工具内部嵌套进程，不再消耗工具名
    tool_processes: BTreeSet<String>,
    pending_commands: VecDeque<CommandEntry>,
    commands_by_process: BTreeMap<String, CommandEntry>,
    completed_unattributed: VecDeque<DeferredExecution>,
    windows: BTreeMap<WindowKey, WindowState>,
    last_indeterminate_alert: BTreeMap<String, u64>,
    last_active_ms: u64,
    terminal_ms: Option<u64>,
}

impl TraceState {
    fn new(now_ms: u64) -> Self {
        Self {
            agent_process_id: None,
            pending_tool_calls: VecDeque::new(),
            seen_tool_calls: BTreeMap::new(),
            tool_processes: BTreeSet::new(),
            pending_commands: VecDeque::new(),
            commands_by_process: BTreeMap::new(),
            completed_unattributed: VecDeque::new(),
            windows: BTreeMap::new(),
            last_indeterminate_alert: BTreeMap::new(),
            last_active_ms: now_ms,
            terminal_ms: None,
        }
    }

    fn observe_agent_identity(&mut self, action: &SemanticActionRecord) {
        if let Some(pid) = find_attr(&action.attributes, ATTR_PROCESS_ID) {
            self.agent_process_id = Some(pid.to_string());
        }
    }

    #[cfg(test)]
    fn observe_llm_response(&mut self, action: &SemanticActionRecord, cap: usize) {
        let cfg = PluginConfig::default();
        let _ = self.observe_llm_response_at(action, &cfg, self.last_active_ms, cap);
    }

    fn observe_llm_response_at(
        &mut self,
        action: &SemanticActionRecord,
        cfg: &PluginConfig,
        now_ms: u64,
        cap: usize,
    ) -> Vec<DeferredExecution> {
        let attribution_ttl_ms = cfg
            .resources
            .attribution_grace_seconds
            .saturating_mul(1000)
            .max(1000);
        self.pending_tool_calls
            .retain(|call| now_ms.saturating_sub(call.observed_ms) <= attribution_ttl_ms);
        self.seen_tool_calls
            .retain(|_, observed_ms| now_ms.saturating_sub(*observed_ms) <= attribution_ttl_ms);

        let Some(json) = find_attr(&action.attributes, ATTR_LLM_TOOL_CALLS_JSON) else {
            return Vec::new();
        };
        let Ok(calls) = serde_json::from_str::<Vec<serde_json::Value>>(json) else {
            return Vec::new();
        };
        for (index, call) in calls.into_iter().enumerate() {
            let Some(name) = call
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
            else {
                continue;
            };
            let hint = tool_call_hint(&call);
            let identity = call
                .get("id")
                .and_then(|id| id.as_str())
                .filter(|id| !id.is_empty())
                .map(|id| format!("id:{id}"))
                .unwrap_or_else(|| {
                    format!(
                        "action:{}:index:{index}:name:{name}:hint:{hint}",
                        action.action_id
                    )
                });
            if self.seen_tool_calls.contains_key(&identity) {
                continue;
            }
            if self.seen_tool_calls.len() >= cap {
                if let Some(oldest) = self
                    .seen_tool_calls
                    .iter()
                    .min_by_key(|(_, observed_ms)| **observed_ms)
                    .map(|(identity, _)| identity.clone())
                {
                    self.seen_tool_calls.remove(&oldest);
                }
            }
            self.seen_tool_calls.insert(identity, now_ms);
            if self.pending_tool_calls.len() >= cap {
                self.pending_tool_calls.pop_front();
            }
            self.pending_tool_calls.push_back(PendingToolCall {
                name: name.to_string(),
                hint,
                observed_ms: now_ms,
            });
        }
        self.reconcile_deferred(cfg, now_ms)
    }

    /// 将流式响应最终化之前已经启动/退出的命令反向绑定到 LLM 工具调用。
    fn reconcile_deferred(&mut self, cfg: &PluginConfig, now_ms: u64) -> Vec<DeferredExecution> {
        let grace_ms = cfg.resources.attribution_grace_seconds.saturating_mul(1000);
        if grace_ms == 0 {
            self.completed_unattributed.clear();
            return Vec::new();
        }
        self.completed_unattributed
            .retain(|execution| now_ms.saturating_sub(execution.observed_ms) <= grace_ms);

        let mut resolved = Vec::new();
        let mut call_index = 0;
        while call_index < self.pending_tool_calls.len() {
            let call = &self.pending_tool_calls[call_index];
            // 历史候选只接受参数提示匹配。无提示 FIFO 仍只用于未来命令，
            // 避免把 OpenCode 的 git 等内部进程误认成工具执行。
            if call.hint.is_empty() {
                call_index += 1;
                continue;
            }
            let call_name = call.name.clone();
            let call_hint = call.hint.clone();
            let call_ms = call.observed_ms;

            let running_process = self
                .commands_by_process
                .iter()
                .filter(|(_, entry)| {
                    entry.deferred_attribution
                        && call_ms.saturating_sub(entry.observed_ms) <= grace_ms
                        && command_matches_hint(entry, &call_hint)
                })
                .min_by_key(|(_, entry)| entry.observed_ms)
                .map(|(process_id, _)| process_id.clone());
            if let Some(process_id) = running_process {
                self.pending_tool_calls.remove(call_index);
                if let Some(entry) = self.commands_by_process.get_mut(&process_id) {
                    entry.tool_name = call_name;
                    entry.monitored = cfg.should_monitor(&entry.tool_name);
                    entry.llm_attributed = true;
                    entry.deferred_attribution = false;
                    if entry.monitored {
                        self.tool_processes.insert(process_id);
                    }
                }
                continue;
            }

            let completed_index = self
                .completed_unattributed
                .iter()
                .enumerate()
                .filter(|(_, execution)| {
                    call_ms.saturating_sub(execution.observed_ms) <= grace_ms
                        && process_result_matches_hint(&execution.result, &call_hint)
                })
                .min_by_key(|(_, execution)| execution.observed_ms)
                .map(|(index, _)| index);
            if let Some(completed_index) = completed_index {
                self.pending_tool_calls.remove(call_index);
                if let Some(mut execution) = self.completed_unattributed.remove(completed_index) {
                    execution.result.tool_name = call_name;
                    execution.result.record = cfg.should_monitor(&execution.result.tool_name);
                    resolved.push(execution);
                }
                continue;
            }
            call_index += 1;
        }
        resolved.sort_by_key(|execution| execution.observed_ms);
        resolved
    }

    /// 登记一次 command.invocation。成败统一由 process.exit 决定。
    /// 只有被判定为“工具执行”的命令才参与统计（monitored=true），
    /// 其余命令仅登记用于 process.exit 关联，避免误报 dropped。
    fn observe_command_invocation(
        &mut self,
        action: &SemanticActionRecord,
        cfg: &PluginConfig,
        cap: usize,
    ) {
        let process_id = find_attr(&action.attributes, ATTR_PROCESS_ID).unwrap_or("");
        if !process_id.is_empty() {
            if let Some(existing) = self.commands_by_process.get(process_id) {
                if existing.llm_attributed {
                    // 工具执行边界已由 LLM 归因确定，exec 替换属于同一次工具执行
                    return;
                }
                if existing.deferred_attribution {
                    // 保留外层工具包装，同时记录 exec 后形态供延迟参数提示匹配。
                    let command_line =
                        find_attr(&action.attributes, ATTR_COMMAND_LINE).unwrap_or("");
                    let executable =
                        find_attr(&action.attributes, ATTR_PROCESS_EXECUTABLE).unwrap_or("");
                    if let Some(existing) = self.commands_by_process.get_mut(process_id) {
                        if !command_line.is_empty()
                            && !existing
                                .match_command_lines
                                .iter()
                                .any(|line| line == command_line)
                        {
                            existing.match_command_lines.push(command_line.to_string());
                        }
                        if !executable.is_empty() {
                            existing.executable = executable.to_string();
                        }
                    }
                    return;
                }
                // 未归因 LLM：进程最终形态为准（bash 对 -c 末命令 exec 成 ls 时，
                // 失败应记到 ls），用新登记覆盖旧登记
                let stale_action_id = existing.action_id.clone();
                self.commands_by_process.remove(process_id);
                self.remove_pending_entry(&stale_action_id);
            }
        }
        let parent_id = find_attr(&action.attributes, ATTR_PROCESS_PARENT_ID);
        let host_tool_name = find_attr(&action.attributes, ATTR_COMMAND_TOOL_NAME);
        let command_line = find_attr(&action.attributes, ATTR_COMMAND_LINE).unwrap_or("");
        let executable = find_attr(&action.attributes, ATTR_PROCESS_EXECUTABLE).unwrap_or("");
        let (monitored, tool_name, llm_attributed, deferred_attribution) = match classify_command(
            self.agent_process_id.as_deref(),
            process_id,
            parent_id,
            host_tool_name,
            !self.pending_tool_calls.is_empty(),
            &self.tool_processes,
            cfg,
        ) {
            CommandClassification::Skip => (false, String::new(), false, false),
            CommandClassification::HostTool(name) => {
                (cfg.should_monitor(&name), name.to_string(), true, false)
            }
            CommandClassification::ConsumeCandidate => {
                let name = take_pending_tool_call(
                    &mut self.pending_tool_calls,
                    command_line,
                    executable,
                    cfg,
                );
                match name {
                    Some(name) => {
                        let monitored = cfg.should_monitor(&name);
                        if monitored && !process_id.is_empty() && self.tool_processes.len() < cap {
                            self.tool_processes.insert(process_id.to_string());
                        }
                        (monitored, name, true, false)
                    }
                    // 队列里可能只有重复流式响应留下的旧提示。当前命令不匹配
                    // 时仍必须进入延迟缓冲，否则它的 process.exit 会被消费，
                    // 后到的正确 LLM response 将再也无法回放这次失败。
                    None if self.agent_process_id.is_some() => (false, String::new(), false, true),
                    None => (false, String::new(), false, false),
                }
            }
            CommandClassification::DeferCandidate => (false, String::new(), false, true),
            CommandClassification::FallbackBasename => {
                let name = find_attr(&action.attributes, ATTR_PROCESS_EXECUTABLE)
                    .and_then(executable_basename)
                    .unwrap_or("unknown")
                    .to_string();
                (cfg.should_monitor(&name), name, false, false)
            }
        };
        let entry = CommandEntry {
            action_id: action.action_id.clone(),
            tool_name,
            process_id: process_id.to_string(),
            command_line: command_line.to_string(),
            match_command_lines: if command_line.is_empty() {
                Vec::new()
            } else {
                vec![command_line.to_string()]
            },
            executable: executable.to_string(),
            observed_ms: self.last_active_ms,
            monitored,
            llm_attributed,
            deferred_attribution,
        };
        if self.pending_commands.len() >= cap {
            if let Some(oldest) = self.pending_commands.pop_front() {
                if !oldest.process_id.is_empty()
                    && self
                        .commands_by_process
                        .get(&oldest.process_id)
                        .is_some_and(|entry| entry.action_id == oldest.action_id)
                {
                    self.commands_by_process.remove(&oldest.process_id);
                }
            }
        }
        self.pending_commands.push_back(entry.clone());
        if !process_id.is_empty() {
            self.commands_by_process
                .insert(process_id.to_string(), entry);
        }
    }

    fn observe_process_exit(
        &mut self,
        action: &SemanticActionRecord,
        cfg: &PluginConfig,
        now_ms: u64,
    ) -> ProcessExitResult {
        let process_id = find_attr(&action.attributes, ATTR_PROCESS_ID).unwrap_or("");
        let exit_code = find_attr(&action.attributes, ATTR_PROCESS_EXIT_CODE).unwrap_or("");
        let failure_summary =
            find_attr(&action.attributes, ATTR_PROCESS_FAILURE_SUMMARY).unwrap_or("");
        if action.status == "in_progress" && exit_code.is_empty() && failure_summary.is_empty() {
            let matched = if process_id.is_empty() {
                !self.pending_commands.is_empty()
            } else {
                self.commands_by_process.contains_key(process_id)
            };
            let mut result = ProcessExitResult::unmatched();
            result.matched = matched;
            return result;
        }
        let entry = if !process_id.is_empty() {
            self.commands_by_process.remove(process_id)
        } else {
            None
        }
        .or_else(|| self.pending_commands.pop_front());
        let Some(entry) = entry else {
            return ProcessExitResult::unmatched();
        };
        self.remove_pending_entry(&entry.action_id);
        if !process_id.is_empty() {
            self.tool_processes.remove(process_id);
        }

        let status = action.status.as_str();
        let (outcome, raw_failure, exit_status, summary) = process_outcome(
            status,
            exit_code,
            failure_summary,
            cfg.alert.unknown_counts_as_success,
        );
        let mut result = ProcessExitResult {
            outcome,
            tool_name: entry.tool_name.clone(),
            raw_failure_type: raw_failure,
            exit_status,
            summary,
            command_line: entry.command_line.clone(),
            match_command_lines: entry.match_command_lines.clone(),
            executable: entry.executable.clone(),
            command_action_id: entry.action_id.clone(),
            matched: true,
            record: entry.monitored && cfg.should_monitor(&entry.tool_name),
        };
        if entry.deferred_attribution {
            result.record = false;
            if self.completed_unattributed.len() >= cfg.resources.pending_queue_capacity {
                self.completed_unattributed.pop_front();
            }
            self.completed_unattributed.push_back(DeferredExecution {
                result: result.clone(),
                exit_action_id: action.action_id.clone(),
                observed_ms: now_ms,
            });
        }
        result
    }

    fn remove_pending_entry(&mut self, action_id: &str) {
        self.pending_commands
            .retain(|entry| entry.action_id != action_id);
    }

    fn record_outcome(
        &mut self,
        tool_name: &str,
        failure_type: &str,
        exit_status: &str,
        outcome: Outcome,
        action_id: &str,
        evidence_ids: &[&str],
        now_ms: u64,
        cfg: &PluginConfig,
        summary: &str,
        command_line: &str,
    ) -> Option<AlertData> {
        if !cfg.should_monitor(tool_name) {
            return None;
        }
        let window_ms = cfg.alert.window_seconds.saturating_mul(1000);
        let key: WindowKey = tool_name.to_string();
        let window = self
            .windows
            .entry(key)
            .or_insert_with(|| WindowState::new(now_ms));
        window.touch(now_ms, window_ms);
        if window.seen(action_id) {
            return None;
        }
        match outcome {
            Outcome::Success => {
                window.success_count = window.success_count.saturating_add(1);
                window.push_evidence(
                    evidence_ids,
                    cfg.evidence.max_count.saturating_mul(4).max(64),
                    cfg.evidence.max_count,
                );
                None
            }
            Outcome::Indeterminate => None,
            Outcome::Failure => {
                window.failure_count = window.failure_count.saturating_add(1);
                *window
                    .breakdown
                    .entry((failure_type.to_string(), exit_status.to_string()))
                    .or_default() += 1;
                if window.failure_count == 1 {
                    // 首个失败证据优先取 command.invocation / mcp.tool_call 动作
                    window.first_action_id = evidence_ids
                        .first()
                        .map(|id| id.to_string())
                        .unwrap_or_else(|| action_id.to_string());
                }
                window.last_action_id = action_id.to_string();
                window.push_evidence(
                    evidence_ids,
                    cfg.evidence.max_count.saturating_mul(4).max(64),
                    cfg.evidence.max_count,
                );
                if !window.should_alert(cfg, now_ms) {
                    return None;
                }
                let alert = window.build_alert_data(tool_name, now_ms, cfg, summary, command_line);
                window.last_alert_ms = now_ms;
                if cfg.alert.after_alert == AfterAlertMode::ResetWindow {
                    window.reset(now_ms);
                }
                Some(alert)
            }
        }
    }

    fn maybe_indeterminate_alert(
        &mut self,
        tool_name: &str,
        action_id: &str,
        reason: &str,
        now_ms: u64,
        cfg: &PluginConfig,
    ) -> Option<IndeterminateData> {
        if cfg.alert.indeterminate_handling != IndeterminateHandling::Diagnostic {
            return None;
        }
        let cooldown_ms = cfg.alert.cooldown_seconds.saturating_mul(1000);
        let last = self
            .last_indeterminate_alert
            .get(tool_name)
            .copied()
            .unwrap_or(0);
        if last > 0 && now_ms.saturating_sub(last) < cooldown_ms {
            return None;
        }
        self.last_indeterminate_alert
            .insert(tool_name.to_string(), now_ms);
        Some(IndeterminateData {
            tool_name: tool_name.to_string(),
            reason: reason.to_string(),
            action_id: action_id.to_string(),
        })
    }
}

#[derive(Clone)]
struct ProcessExitResult {
    outcome: Outcome,
    tool_name: String,
    raw_failure_type: String,
    exit_status: String,
    summary: String,
    command_line: String,
    match_command_lines: Vec<String>,
    executable: String,
    command_action_id: String,
    matched: bool,
    record: bool,
}

impl ProcessExitResult {
    fn unmatched() -> Self {
        Self {
            outcome: Outcome::Indeterminate,
            tool_name: String::new(),
            raw_failure_type: String::new(),
            exit_status: String::new(),
            summary: String::new(),
            command_line: String::new(),
            match_command_lines: Vec::new(),
            executable: String::new(),
            command_action_id: String::new(),
            matched: false,
            record: false,
        }
    }
}

/// 命令是否算作“工具执行”以及工具名来源。
///
/// 优先级：
/// 1. 宿主回填的 `command.tool.name`（agent 自身进程除外）；
/// 2. LLM 工具名队列——非工具进程嵌套子进程的下一条命令即 LLM 发起的工具执行
///    （不要求是 agent 进程的直接子进程，兼容 opencode 等真实拓扑）；
/// 3. `parent_scope=any` 时按可执行文件名兜底（原始命令回归场景）。
///
/// `command.line` 永远不是聚合键。
enum CommandClassification {
    Skip,
    HostTool(String),
    ConsumeCandidate,
    DeferCandidate,
    FallbackBasename,
}

fn classify_command(
    agent_process_id: Option<&str>,
    process_id: &str,
    parent_id: Option<&str>,
    host_tool_name: Option<&str>,
    has_pending_tool_call: bool,
    tool_processes: &BTreeSet<String>,
    cfg: &PluginConfig,
) -> CommandClassification {
    if cfg.filter.tool_scope == ToolScope::McpOnly {
        return CommandClassification::Skip;
    }
    // agent 自身进程不是工具执行。
    if agent_process_id.is_some_and(|agent| agent == process_id) {
        return CommandClassification::Skip;
    }
    if let Some(name) = host_tool_name {
        return CommandClassification::HostTool(name.to_string());
    }
    // 工具进程的嵌套子进程：不是独立工具执行，且不消耗 LLM 工具名
    if parent_id.is_some_and(|parent| tool_processes.contains(parent)) {
        return CommandClassification::Skip;
    }
    if has_pending_tool_call {
        return CommandClassification::ConsumeCandidate;
    }
    if cfg.filter.parent_scope == ParentScope::Any {
        return CommandClassification::FallbackBasename;
    }
    if agent_process_id.is_some() {
        return CommandClassification::DeferCandidate;
    }
    CommandClassification::Skip
}

fn command_matches_hint(entry: &CommandEntry, hint: &str) -> bool {
    entry
        .match_command_lines
        .iter()
        .any(|line| contains_hint(line, hint))
        || contains_hint(&entry.executable, hint)
}

fn process_result_matches_hint(result: &ProcessExitResult, hint: &str) -> bool {
    result
        .match_command_lines
        .iter()
        .any(|line| contains_hint(line, hint))
        || contains_hint(&result.executable, hint)
}

fn process_outcome(
    status: &str,
    exit_code: &str,
    failure_summary: &str,
    unknown_counts_as_success: bool,
) -> (Outcome, String, String, String) {
    if status == "error" {
        let code = if exit_code.is_empty() {
            "error"
        } else {
            exit_code
        };
        let summary = if failure_summary.is_empty() {
            format!("exit code {code}")
        } else {
            failure_summary.to_string()
        };
        return (
            Outcome::Failure,
            RAW_COMMAND_ERROR.to_string(),
            code.to_string(),
            summary,
        );
    }
    if !exit_code.is_empty() && exit_code != "0" {
        let summary = if failure_summary.is_empty() {
            format!("exit code {exit_code}")
        } else {
            failure_summary.to_string()
        };
        return (
            Outcome::Failure,
            RAW_EXIT_CODE_NONZERO.to_string(),
            exit_code.to_string(),
            summary,
        );
    }
    if status == "success" || (status == "unknown" && unknown_counts_as_success) {
        return (
            Outcome::Success,
            String::new(),
            "0".to_string(),
            String::new(),
        );
    }
    let summary = if failure_summary.is_empty() {
        "process exit status indeterminate".to_string()
    } else {
        failure_summary.to_string()
    };
    (
        Outcome::Indeterminate,
        "indeterminate".to_string(),
        "unknown".to_string(),
        summary,
    )
}

/// 从 LLM 工具调用 JSON 中提取参数提示（opencode bash 工具为 arguments.command）。
fn tool_call_hint(call: &serde_json::Value) -> String {
    if let Some(v) = call
        .pointer("/function/arguments_json/command")
        .and_then(|v| v.as_str())
    {
        return v.to_string();
    }
    if let Some(raw) = call.pointer("/function/arguments").and_then(|v| v.as_str()) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) {
            if let Some(cmd) = value.get("command").and_then(|c| c.as_str()) {
                return cmd.to_string();
            }
        }
    }
    String::new()
}

/// 从待匹配工具调用队列中取一个工具名。
///
/// 优先级：
/// 1. 参数提示与命令行的边界匹配（防止 git 等无关进程抢先消耗）；
/// 2. 无提示（其他 agent 格式）的工具调用按 FIFO 兜底。
fn take_pending_tool_call(
    pending: &mut VecDeque<PendingToolCall>,
    command_line: &str,
    executable: &str,
    cfg: &PluginConfig,
) -> Option<String> {
    if let Some(position) = pending.iter().position(|call| {
        !call.hint.is_empty()
            && (contains_hint(command_line, &call.hint) || contains_hint(executable, &call.hint))
    }) {
        return pending.remove(position).map(|call| call.name);
    }
    match cfg.filter.llm_attribution {
        LlmAttribution::Fifo => {
            if pending.iter().any(|call| call.hint.is_empty()) {
                pending.pop_front().map(|call| call.name)
            } else {
                None
            }
        }
        LlmAttribution::SkipIfAmbiguous => {
            if pending.len() == 1 && pending.front().is_some_and(|call| call.hint.is_empty()) {
                pending.pop_front().map(|call| call.name)
            } else {
                None
            }
        }
    }
}

/// 子串匹配并要求边界为空白/串首串尾，避免 `core.fsmonitor=false` 误匹配 `false`。
fn contains_hint(text: &str, hint: &str) -> bool {
    if hint.is_empty() {
        return false;
    }
    let bytes = text.as_bytes();
    let hint_len = hint.len();
    let mut start = 0;
    while let Some(relative) = text[start..].find(hint) {
        let absolute = start + relative;
        let before_ok = absolute == 0 || bytes[absolute - 1].is_ascii_whitespace();
        let after = absolute + hint_len;
        let after_ok = after >= bytes.len() || bytes[after].is_ascii_whitespace();
        if before_ok && after_ok {
            return true;
        }
        start = absolute + 1;
    }
    false
}

fn executable_basename(exec: &str) -> Option<&str> {
    exec.rsplit('/').next().filter(|s| !s.is_empty())
}

// ============================================================================
// 告警数据与 payload
// ============================================================================

struct AlertData {
    tool_name: String,
    /// 窗口内出现次数最多的失败类别
    failure_type: String,
    /// 窗口内出现次数最多的退出状态
    exit_status: String,
    failure_breakdown: Vec<BreakdownItem>,
    failure_count: u64,
    success_count: u64,
    window_start_ms: u64,
    window_end_ms: u64,
    first_action_id: String,
    last_action_id: String,
    evidence_action_ids: Vec<String>,
    evidence_truncated_count: u64,
    summary: String,
    debug_command_line: String,
}

impl WindowState {
    fn build_alert_data(
        &self,
        tool_name: &str,
        now_ms: u64,
        cfg: &PluginConfig,
        summary: &str,
        command_line: &str,
    ) -> AlertData {
        let (failure_type, exit_status) = self.dominant_breakdown();
        let debug_command_line = if cfg.debug_include_command_line {
            cfg.desensitization.desensitize(command_line)
        } else {
            String::new()
        };
        AlertData {
            tool_name: tool_name.to_string(),
            failure_type,
            exit_status,
            failure_breakdown: self.breakdown_items(),
            failure_count: self.failure_count,
            success_count: self.success_count,
            window_start_ms: self.window_start_ms,
            window_end_ms: now_ms,
            first_action_id: self.first_action_id.clone(),
            last_action_id: self.last_action_id.clone(),
            evidence_action_ids: self.failure_evidence.clone(),
            evidence_truncated_count: self.failure_evidence_truncated,
            summary: cfg.desensitization.desensitize(summary),
            debug_command_line,
        }
    }

    /// 窗口内出现次数最多的 (失败类型, 退出状态)。
    fn dominant_breakdown(&self) -> (String, String) {
        let mut best: Option<(&str, &str, u64)> = None;
        for ((failure_type, exit_status), count) in &self.breakdown {
            if best.is_none_or(|(_, _, best_count)| *count > best_count) {
                best = Some((failure_type.as_str(), exit_status.as_str(), *count));
            }
        }
        best.map(|(ft, es, _)| (ft.to_string(), es.to_string()))
            .unwrap_or_default()
    }

    fn breakdown_items(&self) -> Vec<BreakdownItem> {
        self.breakdown
            .iter()
            .map(|((failure_type, exit_status), count)| BreakdownItem {
                failure_type: failure_type.clone(),
                exit_status: exit_status.clone(),
                count: *count,
            })
            .collect()
    }
}

struct IndeterminateData {
    tool_name: String,
    reason: String,
    action_id: String,
}

impl DesensitizationConfig {
    fn desensitize(&self, raw: &str) -> String {
        match self.mode {
            DesensitizationMode::CategoryOnly => String::new(),
            DesensitizationMode::Raw => raw.to_string(),
            DesensitizationMode::Sanitized => {
                let mut out = redact_keywords(raw, &self.redact_keywords);
                out = redact_secret_runs(&out);
                truncate_chars(&out, self.summary_max_chars)
            }
        }
    }
}

fn redact_keywords(raw: &str, keywords: &[String]) -> String {
    let mut out = raw.to_string();
    for keyword in keywords {
        if keyword.is_empty() {
            continue;
        }
        while let Some(pos) = out.find(keyword.as_str()) {
            let end = pos + keyword.len();
            out.replace_range(pos..end, "***");
        }
    }
    out
}

/// 简易密钥片段脱敏（不依赖 regex）：长 hex 串、长 base64 串、PEM 标记。
fn redact_secret_runs(raw: &str) -> String {
    let chars: Vec<char> = raw.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_ascii_hexdigit() {
            let start = i;
            while i < chars.len() && chars[i].is_ascii_hexdigit() {
                i += 1;
            }
            if i - start >= 16 {
                out.push_str("***");
            } else {
                out.extend(&chars[start..i]);
            }
            continue;
        }
        if c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=' {
            let start = i;
            while i < chars.len()
                && (chars[i].is_ascii_alphanumeric()
                    || chars[i] == '+'
                    || chars[i] == '/'
                    || chars[i] == '=')
            {
                i += 1;
            }
            if i - start >= 24 {
                out.push_str("***");
            } else {
                out.extend(&chars[start..i]);
            }
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

fn truncate_chars(raw: &str, max_chars: usize) -> String {
    if raw.chars().count() <= max_chars {
        return raw.to_string();
    }
    raw.chars().take(max_chars).collect()
}

#[derive(Serialize)]
struct FrequentFailurePayload {
    alert_type: &'static str,
    timestamp: String,
    trace_id: String,
    tool_name: String,
    failure_type: String,
    exit_status: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    failure_breakdown: Vec<BreakdownItem>,
    failure_count: u64,
    total_count: u64,
    failure_rate: f64,
    threshold: ThresholdPayload,
    window: WindowPayload,
    first_action_id: String,
    last_action_id: String,
    evidence_action_ids: Vec<String>,
    evidence_truncated_count: u64,
    #[serde(skip_serializing_if = "String::is_empty")]
    summary: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    debug_command_line: String,
}

#[derive(Clone, Serialize)]
struct BreakdownItem {
    failure_type: String,
    exit_status: String,
    count: u64,
}

#[derive(Serialize)]
struct ThresholdPayload {
    min_failure_count: u64,
    min_failure_rate: f64,
    window_seconds: u64,
    trigger_mode: String,
}

#[derive(Serialize)]
struct WindowPayload {
    start_ms: u64,
    end_ms: u64,
}

#[derive(Serialize)]
struct IndeterminatePayload {
    alert_type: &'static str,
    timestamp: String,
    trace_id: String,
    tool_name: String,
    reason: String,
    action_id: String,
}

fn build_frequent_failure_payload(alert: &AlertData, trace_id: &str, cfg: &PluginConfig) -> String {
    let total = alert.failure_count + alert.success_count;
    let rate = if total > 0 {
        alert.failure_count as f64 / total as f64
    } else {
        0.0
    };
    let payload = FrequentFailurePayload {
        alert_type: "frequent_failure",
        timestamp: epoch_ms_to_iso8601(alert.window_end_ms),
        trace_id: trace_id.to_string(),
        tool_name: alert.tool_name.clone(),
        failure_type: alert.failure_type.clone(),
        exit_status: alert.exit_status.clone(),
        failure_breakdown: alert.failure_breakdown.clone(),
        failure_count: alert.failure_count,
        total_count: total,
        failure_rate: rate,
        threshold: ThresholdPayload {
            min_failure_count: cfg.alert.min_failure_count,
            min_failure_rate: cfg.alert.min_failure_rate,
            window_seconds: cfg.alert.window_seconds,
            trigger_mode: cfg.alert.trigger_mode.as_str().to_string(),
        },
        window: WindowPayload {
            start_ms: alert.window_start_ms,
            end_ms: alert.window_end_ms,
        },
        first_action_id: alert.first_action_id.clone(),
        last_action_id: alert.last_action_id.clone(),
        evidence_action_ids: alert.evidence_action_ids.clone(),
        evidence_truncated_count: alert.evidence_truncated_count,
        summary: alert.summary.clone(),
        debug_command_line: alert.debug_command_line.clone(),
    };
    serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string())
}

fn build_indeterminate_payload(data: &IndeterminateData, trace_id: &str, now_ms: u64) -> String {
    let payload = IndeterminatePayload {
        alert_type: "indeterminate_result",
        timestamp: epoch_ms_to_iso8601(now_ms),
        trace_id: trace_id.to_string(),
        tool_name: data.tool_name.clone(),
        reason: data.reason.clone(),
        action_id: data.action_id.clone(),
    };
    serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string())
}

fn dedup_key_for(tool_name: &str, failure_type: &str, window_start_ms: u64) -> String {
    let raw = format!("freq:{tool_name}:{failure_type}:{window_start_ms}");
    if raw.len() <= 200 {
        raw
    } else {
        stable_hash(&raw)
    }
}

fn stable_hash(input: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in input.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("freq-hash:{hash:016x}")
}

fn submit_alert(
    trace_id: &str,
    definition_key: &str,
    payload_json: String,
    dedup_key: Option<String>,
) {
    let Ok(ctx) = actrail::plugin::observation_context_read::trace_context_get() else {
        return;
    };
    let alert_token = ctx.alert_token.unwrap_or_default();
    let request = AlertWriteRequest {
        trace_id: trace_id.to_string(),
        alert_token,
        draft: AlertDraft {
            definition_key: definition_key.to_string(),
            payload_json,
            deduplication_key: dedup_key,
        },
    };
    let _ = actrail::plugin::alert_write::submit(&request);
}

// ============================================================================
// 插件主体
// ============================================================================

fn collect_process_result(
    trace: &mut TraceState,
    result: ProcessExitResult,
    exit_action_id: &str,
    now_ms: u64,
    config: &PluginConfig,
    trace_id: &str,
    alerts: &mut Vec<(String, String, Option<String>)>,
) {
    if !result.record {
        return;
    }
    let failure_type = config.map_failure_type(&result.raw_failure_type);
    let evidence_ids = [result.command_action_id.as_str(), exit_action_id];
    if let Some(alert) = trace.record_outcome(
        &result.tool_name,
        &failure_type,
        &result.exit_status,
        result.outcome,
        exit_action_id,
        &evidence_ids,
        now_ms,
        config,
        &result.summary,
        &result.command_line,
    ) {
        let key = dedup_key_for(&alert.tool_name, &alert.failure_type, alert.window_start_ms);
        let payload = build_frequent_failure_payload(&alert, trace_id, config);
        alerts.push((ALERT_KEY_FREQUENT_FAILURE.to_string(), payload, Some(key)));
    } else if result.outcome == Outcome::Indeterminate {
        if let Some(data) = trace.maybe_indeterminate_alert(
            &result.tool_name,
            exit_action_id,
            "process exit status indeterminate",
            now_ms,
            config,
        ) {
            let payload = build_indeterminate_payload(&data, trace_id, now_ms);
            alerts.push((ALERT_KEY_INDETERMINATE.to_string(), payload, None));
        }
    }
}

struct PluginState {
    config: PluginConfig,
    traces: BTreeMap<String, TraceState>,
    fallback_clock_ms: u64,
}

impl PluginState {
    fn load() -> Result<Self, String> {
        Ok(Self {
            config: PluginConfig::load()?,
            traces: BTreeMap::new(),
            fallback_clock_ms: 0,
        })
    }

    fn now_ms(&mut self) -> u64 {
        match actrail::plugin::observation_context_read::current_time_ms() {
            Ok(ms) => ms,
            Err(_) => {
                self.fallback_clock_ms = self.fallback_clock_ms.wrapping_add(1);
                self.fallback_clock_ms
            }
        }
    }

    fn consume_batch(&mut self, batch: &ObservationBatch) -> Result<ObservationReport, String> {
        let now_ms = self.now_ms();
        let trace_id = batch.trace_id.clone();
        let mut dropped: u64 = 0;
        let mut alerts: Vec<(String, String, Option<String>)> = Vec::new();

        {
            let config = &self.config;
            let max_states = config.resources.max_trace_states;
            let queue_cap = config.resources.pending_queue_capacity;
            let trace = trace_mut(&mut self.traces, &trace_id, now_ms, max_states);
            trace.last_active_ms = now_ms;

            // 第一遍：先登记 agent 身份与 LLM 工具名，避免同 batch 内顺序颠倒；
            // 同时回放早于流式响应最终化的工具执行结果。
            let mut resolved_deferred = Vec::new();
            for action in &batch.semantic_actions {
                match action.kind.as_str() {
                    "agent.identity" => trace.observe_agent_identity(action),
                    "llm.response" => resolved_deferred
                        .extend(trace.observe_llm_response_at(action, config, now_ms, queue_cap)),
                    _ => {}
                }
            }
            for execution in resolved_deferred {
                collect_process_result(
                    trace,
                    execution.result,
                    &execution.exit_action_id,
                    now_ms,
                    config,
                    &trace_id,
                    &mut alerts,
                );
            }

            // 第二遍：处理工具调用/结果/进程退出/策略决策
            for action in &batch.semantic_actions {
                match action.kind.as_str() {
                    "agent.identity" | "llm.response" => {}
                    "mcp.tool_call" => {
                        let (outcome, tool_name, exit_status, summary) = mcp_tool_outcome(action);
                        match outcome {
                            Outcome::Failure => {
                                let failure_type = config.map_failure_type(RAW_MCP_ERROR);
                                if let Some(alert) = trace.record_outcome(
                                    &tool_name,
                                    &failure_type,
                                    &exit_status,
                                    Outcome::Failure,
                                    &action.action_id,
                                    &[&action.action_id],
                                    now_ms,
                                    config,
                                    &summary,
                                    "",
                                ) {
                                    let key = dedup_key_for(
                                        &alert.tool_name,
                                        &alert.failure_type,
                                        alert.window_start_ms,
                                    );
                                    let payload =
                                        build_frequent_failure_payload(&alert, &trace_id, config);
                                    alerts.push((
                                        ALERT_KEY_FREQUENT_FAILURE.to_string(),
                                        payload,
                                        Some(key),
                                    ));
                                }
                            }
                            Outcome::Success => {
                                trace.record_outcome(
                                    &tool_name,
                                    "mcp_success",
                                    &exit_status,
                                    Outcome::Success,
                                    &action.action_id,
                                    &[&action.action_id],
                                    now_ms,
                                    config,
                                    "",
                                    "",
                                );
                            }
                            Outcome::Indeterminate => {
                                if let Some(data) = trace.maybe_indeterminate_alert(
                                    &tool_name,
                                    &action.action_id,
                                    "mcp execution status indeterminate",
                                    now_ms,
                                    config,
                                ) {
                                    let payload =
                                        build_indeterminate_payload(&data, &trace_id, now_ms);
                                    alerts.push((
                                        ALERT_KEY_INDETERMINATE.to_string(),
                                        payload,
                                        None,
                                    ));
                                }
                            }
                        }
                    }
                    "command.invocation" => {
                        trace.observe_command_invocation(action, config, queue_cap);
                    }
                    "process.exit" => {
                        let result = trace.observe_process_exit(action, config, now_ms);
                        if !result.matched {
                            dropped = dropped.saturating_add(1);
                        }
                        collect_process_result(
                            trace,
                            result,
                            &action.action_id,
                            now_ms,
                            config,
                            &trace_id,
                            &mut alerts,
                        );
                    }
                    "enforcement.decision" => {
                        let (outcome, tool_name, exit_status, summary) =
                            enforcement_outcome(action);
                        match outcome {
                            Outcome::Failure => {
                                let failure_type = config.map_failure_type(RAW_POLICY_DENIED);
                                if let Some(alert) = trace.record_outcome(
                                    &tool_name,
                                    &failure_type,
                                    &exit_status,
                                    Outcome::Failure,
                                    &action.action_id,
                                    &[&action.action_id],
                                    now_ms,
                                    config,
                                    &summary,
                                    "",
                                ) {
                                    let key = dedup_key_for(
                                        &alert.tool_name,
                                        &alert.failure_type,
                                        alert.window_start_ms,
                                    );
                                    let payload =
                                        build_frequent_failure_payload(&alert, &trace_id, config);
                                    alerts.push((
                                        ALERT_KEY_FREQUENT_FAILURE.to_string(),
                                        payload,
                                        Some(key),
                                    ));
                                }
                            }
                            Outcome::Success => {
                                trace.record_outcome(
                                    &tool_name,
                                    "policy_allowed",
                                    &exit_status,
                                    Outcome::Success,
                                    &action.action_id,
                                    &[&action.action_id],
                                    now_ms,
                                    config,
                                    "",
                                    "",
                                );
                            }
                            Outcome::Indeterminate => {
                                if let Some(data) = trace.maybe_indeterminate_alert(
                                    &tool_name,
                                    &action.action_id,
                                    "enforcement decision indeterminate",
                                    now_ms,
                                    config,
                                ) {
                                    let payload =
                                        build_indeterminate_payload(&data, &trace_id, now_ms);
                                    alerts.push((
                                        ALERT_KEY_INDETERMINATE.to_string(),
                                        payload,
                                        None,
                                    ));
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        for (definition_key, payload, dedup_key) in alerts {
            submit_alert(&trace_id, &definition_key, payload, dedup_key);
        }

        // trace 终态后仍可能补送 process.exit 或最终化的流式 LLM 动作。
        // 保留一个归因宽限期，不能在 lifecycle batch 到达时立即删除状态。
        use actrail::plugin::types::TraceLifecycleState;
        if let Some(state) = &batch.lifecycle_transition {
            if matches!(
                state,
                TraceLifecycleState::Completed
                    | TraceLifecycleState::Exited
                    | TraceLifecycleState::Failed
            ) {
                if let Some(trace) = self.traces.get_mut(&trace_id) {
                    trace.terminal_ms = Some(now_ms);
                }
            }
        }

        // 活跃 trace 按 state TTL 回收；终态 trace 按较短的归因宽限期回收。
        let ttl_ms = self.config.resources.state_ttl_seconds.saturating_mul(1000);
        let terminal_grace_ms = self
            .config
            .resources
            .attribution_grace_seconds
            .saturating_mul(1000)
            .max(1000);
        self.traces.retain(|_, trace| {
            if let Some(terminal_ms) = trace.terminal_ms {
                now_ms.saturating_sub(terminal_ms) < terminal_grace_ms
            } else {
                ttl_ms == 0 || now_ms.saturating_sub(trace.last_active_ms) < ttl_ms
            }
        });

        Ok(ObservationReport {
            observed_records: batch.semantic_actions.len() as u64,
            dropped_records: dropped,
        })
    }
}

fn trace_mut<'a>(
    traces: &'a mut BTreeMap<String, TraceState>,
    trace_id: &str,
    now_ms: u64,
    max_states: usize,
) -> &'a mut TraceState {
    if !traces.contains_key(trace_id) {
        if traces.len() >= max_states {
            let victim = traces
                .iter()
                .min_by_key(|(_, trace)| trace.last_active_ms)
                .map(|(key, _)| key.clone());
            if let Some(victim) = victim {
                traces.remove(&victim);
            }
        }
        traces.insert(trace_id.to_string(), TraceState::new(now_ms));
    }
    traces
        .get_mut(trace_id)
        .expect("trace state was just inserted")
}

// ============================================================================
// 事件 → 成败/工具维度 归一
// ============================================================================

fn mcp_tool_outcome(action: &SemanticActionRecord) -> (Outcome, String, String, String) {
    let tool_name = find_attr(&action.attributes, ATTR_MCP_TOOL_NAME).unwrap_or("unknown");
    let execution_status = find_attr(&action.attributes, ATTR_MCP_EXECUTION_STATUS).unwrap_or("");
    let status = action.status.as_str();
    let (outcome, exit_status) = if status == "error" || execution_status == "error" {
        (Outcome::Failure, "mcp:error".to_string())
    } else if status == "success" || execution_status == "success" {
        (Outcome::Success, "mcp:success".to_string())
    } else {
        (Outcome::Indeterminate, "mcp:unknown".to_string())
    };
    let summary = if outcome == Outcome::Failure {
        "mcp tool call failed".to_string()
    } else {
        String::new()
    };
    (outcome, tool_name.to_string(), exit_status, summary)
}

fn enforcement_outcome(action: &SemanticActionRecord) -> (Outcome, String, String, String) {
    let operation = find_attr(&action.attributes, ATTR_ENFORCEMENT_OPERATION).unwrap_or("decision");
    let decision = find_attr(&action.attributes, ATTR_ENFORCEMENT_DECISION).unwrap_or("");
    let result = find_attr(&action.attributes, ATTR_ENFORCEMENT_RESULT).unwrap_or("");
    let status = action.status.as_str();
    let tool_name = format!("enforcement:{operation}");
    let (outcome, exit_status) = match result {
        "denied" | "deny" | "blocked" | "error" => (Outcome::Failure, "policy:deny".to_string()),
        "allowed" | "allow" | "success" => (Outcome::Success, "policy:allow".to_string()),
        _ if status == "error" => (Outcome::Failure, "policy:deny".to_string()),
        _ if status == "success" => (Outcome::Success, "policy:allow".to_string()),
        _ => (Outcome::Indeterminate, "policy:unknown".to_string()),
    };
    let summary = if outcome == Outcome::Failure {
        format!("enforcement {} -> {}", operation, decision)
    } else {
        String::new()
    };
    (outcome, tool_name, exit_status, summary)
}

// ============================================================================
// 工具函数
// ============================================================================

fn find_attr<'a>(
    attributes: &'a [actrail::plugin::types::AttributePair],
    key: &str,
) -> Option<&'a str> {
    attributes
        .iter()
        .find(|attr| attr.key == key)
        .map(|attr| attr.value.as_str())
}

/// 将 epoch 毫秒转换为 ISO 8601 UTC 字符串（YYYY-MM-DDTHH:MM:SS.mmmZ）
fn epoch_ms_to_iso8601(epoch_ms: u64) -> String {
    const DAYS_IN_MONTH: [u32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

    let secs = epoch_ms / 1000;
    let millis = epoch_ms % 1000;
    let total_days = secs / 86400;
    let remaining_secs = secs % 86400;
    let hours = remaining_secs / 3600;
    let minutes = (remaining_secs % 3600) / 60;
    let seconds = remaining_secs % 60;

    let mut days_left = total_days;
    let mut year = 1970u32;
    loop {
        let days_in_year: u32 = if is_leap_year(year) { 366 } else { 365 };
        if days_left < u64::from(days_in_year) {
            break;
        }
        days_left -= u64::from(days_in_year);
        year += 1;
    }

    let mut month = 1u32;
    for m in 0..12u32 {
        month = m + 1;
        let dim = if m == 1 && is_leap_year(year) {
            29
        } else {
            DAYS_IN_MONTH[m as usize]
        };
        if days_left < u64::from(dim) {
            break;
        }
        days_left -= u64::from(dim);
    }
    let day = days_left + 1;

    let mut buf = String::with_capacity(24);
    let _ = write!(
        buf,
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        year, month, day, hours, minutes, seconds, millis
    );
    buf
}

fn is_leap_year(year: u32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

// ============================================================================
// WIT Component 实现
// ============================================================================

#[global_allocator]
static ALLOCATOR: wee_alloc::WeeAlloc = wee_alloc::WeeAlloc::INIT;

struct Component;

static STATE: Mutex<Option<PluginState>> = Mutex::new(None);

impl Guest for Component {
    fn consume(batch: ObservationBatch) -> Result<ObservationReport, String> {
        wit_bindgen::rt::maybe_link_cabi_realloc();
        let mut guard = STATE.lock();
        if guard.is_none() {
            *guard = Some(PluginState::load()?);
        }
        let plugin = guard.as_mut().expect("plugin state initialized");
        plugin.consume_batch(&batch)
    }
}

#[cfg(not(test))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cabi_realloc(
    old_ptr: *mut u8,
    old_len: usize,
    align: usize,
    new_len: usize,
) -> *mut u8 {
    use alloc::alloc::{Layout, alloc, realloc};
    let layout;
    let ptr = unsafe {
        if old_len == 0 {
            if new_len == 0 {
                return align as *mut u8;
            }
            layout = Layout::from_size_align_unchecked(new_len, align);
            alloc(layout)
        } else {
            layout = Layout::from_size_align_unchecked(old_len, align);
            realloc(old_ptr, layout, new_len)
        }
    };
    if ptr.is_null() {
        core::arch::wasm32::unreachable();
    }
    ptr
}

#[cfg(not(test))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memcmp(left: *const u8, right: *const u8, len: usize) -> i32 {
    let mut index = 0;
    while index < len {
        let left_byte = unsafe { *left.add(index) };
        let right_byte = unsafe { *right.add(index) };
        if left_byte != right_byte {
            return i32::from(left_byte) - i32::from(right_byte);
        }
        index += 1;
    }
    0
}

#[cfg(not(test))]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {}
}

#[cfg(not(test))]
export!(Component);

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> PluginConfig {
        PluginConfig::default()
    }

    fn record(
        trace: &mut TraceState,
        cfg: &PluginConfig,
        tool: &str,
        action_id: &str,
        outcome: Outcome,
        now_ms: u64,
    ) -> Option<AlertData> {
        trace.record_outcome(
            tool,
            "runtime_error",
            "2",
            outcome,
            action_id,
            &[action_id],
            now_ms,
            cfg,
            "exit code 2",
            "",
        )
    }

    #[test]
    fn three_failures_trigger_alert_and_reset() {
        let mut trace = TraceState::new(1000);
        let cfg = test_config();
        let mut alert = None;
        for i in 0..3 {
            alert = record(
                &mut trace,
                &cfg,
                "bash",
                &format!("f{i}"),
                Outcome::Failure,
                1000 + i,
            );
        }
        let alert = alert.expect("3 failures must alert");
        assert_eq!(alert.failure_count, 3);
        assert_eq!(alert.total_count_for_test(), 3);
        assert_eq!(alert.evidence_action_ids.len(), 3);
        // 窗口已重置：继续失败属于新一轮
        assert!(record(&mut trace, &cfg, "bash", "f3", Outcome::Failure, 1010).is_none());
    }

    #[test]
    fn success_counts_in_rate_denominator() {
        let mut trace = TraceState::new(1000);
        let mut cfg = test_config();
        cfg.alert.trigger_mode = TriggerMode::CountAndRate;
        cfg.alert.min_failure_rate = 0.5;
        // 交错事件，第 3 次失败时已有 4 次成功 → rate = 3/7 ≈ 0.43 < 0.5，不告警
        let events: &[(Outcome, &str, u64)] = &[
            (Outcome::Failure, "f0", 1000),
            (Outcome::Success, "s0", 1001),
            (Outcome::Success, "s1", 1002),
            (Outcome::Success, "s2", 1003),
            (Outcome::Failure, "f1", 1004),
            (Outcome::Success, "s3", 1005),
            (Outcome::Failure, "f2", 1006),
        ];
        for (outcome, id, t) in events {
            assert!(
                record(&mut trace, &cfg, "bash", id, *outcome, *t).is_none(),
                "rate 0.43 must not alert yet"
            );
        }
        // 第 4 次失败后 4F/4S → rate = 0.5 → 告警
        let alert = record(&mut trace, &cfg, "bash", "f3", Outcome::Failure, 1007)
            .expect("rate 0.5 must alert");
        assert_eq!(alert.failure_count, 4);
        assert_eq!(alert.success_count, 4);
    }

    #[test]
    fn low_rate_does_not_alert_in_and_mode() {
        let mut trace = TraceState::new(1000);
        let mut cfg = test_config();
        cfg.alert.trigger_mode = TriggerMode::CountAndRate;
        cfg.alert.min_failure_rate = 0.5;
        let events: &[(Outcome, &str, u64)] = &[
            (Outcome::Failure, "f0", 1000),
            (Outcome::Success, "s0", 1001),
            (Outcome::Success, "s1", 1002),
            (Outcome::Success, "s2", 1003),
            (Outcome::Failure, "f1", 1004),
            (Outcome::Success, "s3", 1005),
            (Outcome::Success, "s4", 1006),
            (Outcome::Failure, "f2", 1007),
        ];
        for (outcome, id, t) in events {
            assert!(
                record(&mut trace, &cfg, "bash", id, *outcome, *t).is_none(),
                "rate 0.375 must not alert"
            );
        }
        // 3F/5S 未告警；第 4 次失败后 4F/5S ≈ 0.44 < 0.5 仍不告警
        assert!(record(&mut trace, &cfg, "bash", "f3", Outcome::Failure, 1008).is_none());
    }

    #[test]
    fn cooldown_suppresses_after_reset() {
        let mut trace = TraceState::new(1000);
        let cfg = test_config();
        let mut alert = None;
        for i in 0..3 {
            alert = record(
                &mut trace,
                &cfg,
                "bash",
                &format!("f{i}"),
                Outcome::Failure,
                1000 + i,
            );
        }
        assert!(alert.is_some(), "first burst alerts");
        // 冷却期内（告警于 t=1002 触发，60s 冷却）不重复告警
        assert!(
            record(
                &mut trace,
                &cfg,
                "bash",
                "f3",
                Outcome::Failure,
                1002 + 30_000
            )
            .is_none()
        );
        // 冷却结束后，新的窗口需要重新累积 3 次失败
        let mut alert = None;
        for i in 4..7 {
            alert = record(
                &mut trace,
                &cfg,
                "bash",
                &format!("f{i}"),
                Outcome::Failure,
                1002 + 60_001 + (i - 4),
            );
        }
        assert!(alert.is_some(), "fresh burst after cooldown alerts");
    }

    #[test]
    fn indeterminate_never_alerts_as_failure() {
        let mut trace = TraceState::new(1000);
        let cfg = test_config();
        for i in 0..10 {
            assert!(
                record(
                    &mut trace,
                    &cfg,
                    "bash",
                    &format!("u{i}"),
                    Outcome::Indeterminate,
                    1000 + i,
                )
                .is_none(),
                "indeterminate must not fabricate a failure alert"
            );
        }
    }

    #[test]
    fn desensitization_sanitizes_secrets() {
        let cfg = DesensitizationConfig {
            mode: DesensitizationMode::Sanitized,
            summary_max_chars: 200,
            redact_keywords: vec!["sk-".to_string(), "token".to_string(), "secret".to_string()],
        };
        let raw = "failed sk-abcdef123456 token=xyz path /home/user/secret.txt";
        let out = cfg.desensitize(raw);
        assert!(!out.contains("sk-"), "sk- prefix must be redacted");
        assert!(!out.contains("token"), "token must be redacted");
        assert!(!out.contains("secret"), "secret must be redacted");

        let category_only = DesensitizationConfig {
            mode: DesensitizationMode::CategoryOnly,
            ..DesensitizationConfig::default()
        };
        assert_eq!(category_only.desensitize(raw), "");
    }

    #[test]
    fn redact_long_hex_and_base64_runs() {
        let hex = "0123456789abcdef0123456789abcdef";
        let out = redact_secret_runs(hex);
        assert_eq!(out, "***");
        let short = "abc123";
        assert_eq!(redact_secret_runs(short), short);
        let b64 = "QUJDREVGR0hJSktMTU5PUFFSU1RVVldYWVo=";
        assert_eq!(redact_secret_runs(b64), "***");
    }

    #[test]
    fn pattern_matching_supports_suffix_wildcard() {
        assert!(pattern_matches("bash", "bash"));
        assert!(pattern_matches("bash", "ba*"));
        assert!(!pattern_matches("bash", "read"));
        assert!(!pattern_matches("bash", "b*zz"));
    }

    #[test]
    fn dedup_key_stays_bounded() {
        let short = dedup_key_for("bash", "runtime_error", 1786089600000);
        assert!(short.len() <= 256);
        assert!(short.contains("freq:"));
        let long_tool = "x".repeat(500);
        let hashed = dedup_key_for(&long_tool, "runtime_error", 1786089600000);
        assert!(hashed.len() <= 256);
        assert!(hashed.starts_with("freq-hash:"));
    }

    #[test]
    fn iso8601_formatting() {
        assert_eq!(
            epoch_ms_to_iso8601(1786089600000),
            "2026-08-07T08:00:00.000Z"
        );
        assert_eq!(epoch_ms_to_iso8601(0), "1970-01-01T00:00:00.000Z");
    }

    #[test]
    fn e2e_config_parses_and_validates() {
        let path = concat!(
            "../../../../tests/v2/regression/tool_frequent_failure_alert/",
            "tool-frequent-failure-alert.e2e.config.json"
        );
        let raw =
            std::fs::read_to_string(path).unwrap_or_else(|error| panic!("read {path}: {error}"));
        let parsed = serde_json::from_str::<PluginConfig>(&raw)
            .unwrap_or_else(|error| panic!("parse {path}: {error}"));
        parsed
            .validate()
            .unwrap_or_else(|error| panic!("validate {path}: {error}"));
        assert_eq!(parsed.reporting.mode, "database");
    }

    fn action_record(kind: &str, attrs: &[(&str, &str)]) -> SemanticActionRecord {
        SemanticActionRecord {
            action_id: format!("action-{}", kind),
            trace_id: "trace-test".to_string(),
            kind: kind.to_string(),
            status: "success".to_string(),
            completeness: "complete".to_string(),
            file_change: None,
            attributes: attrs
                .iter()
                .map(|(k, v)| actrail::plugin::types::AttributePair {
                    key: k.to_string(),
                    value: v.to_string(),
                })
                .collect(),
        }
    }

    #[test]
    fn llm_queue_is_authoritative_regardless_of_parent_topology() {
        let cfg = test_config(); // 默认 llm_and_mcp + agent_child
        let tool_processes = BTreeSet::new();
        // pending 工具名存在、parent 不是 agent 进程 → 仍判定为 LLM 工具执行
        assert!(matches!(
            classify_command(
                Some("100"),
                "200",
                Some("999"),
                None,
                true,
                &tool_processes,
                &cfg,
            ),
            CommandClassification::ConsumeCandidate
        ));
        // 嵌套在工具进程内的子进程 → 跳过且不消耗工具名
        let mut nested = BTreeSet::new();
        nested.insert("200".to_string());
        assert!(matches!(
            classify_command(Some("100"), "201", Some("200"), None, true, &nested, &cfg,),
            CommandClassification::Skip
        ));
    }

    #[test]
    fn host_backfill_and_agent_self_rules() {
        let cfg = test_config();
        let tool_processes = BTreeSet::new();
        // 宿主回填 command.tool.name → 权威采用
        assert!(matches!(
            classify_command(
                Some("100"),
                "200",
                None,
                Some("bash"),
                false,
                &tool_processes,
                &cfg,
            ),
            CommandClassification::HostTool(name) if name == "bash"
        ));
        // agent 自身进程即使带宿主回填也不算工具执行
        assert!(matches!(
            classify_command(
                Some("100"),
                "100",
                None,
                Some("bash"),
                false,
                &tool_processes,
                &cfg,
            ),
            CommandClassification::Skip
        ));
        // 无 pending、agent_child → 延迟归因候选；any → 按可执行文件名兜底
        assert!(matches!(
            classify_command(Some("100"), "200", None, None, false, &tool_processes, &cfg,),
            CommandClassification::DeferCandidate
        ));
        let mut any_cfg = test_config();
        any_cfg.filter.parent_scope = ParentScope::Any;
        assert!(matches!(
            classify_command(
                Some("100"),
                "200",
                None,
                None,
                false,
                &tool_processes,
                &any_cfg,
            ),
            CommandClassification::FallbackBasename
        ));
        // mcp_only：命令一律不统计
        let mut mcp_cfg = test_config();
        mcp_cfg.filter.tool_scope = ToolScope::McpOnly;
        assert!(matches!(
            classify_command(
                Some("100"),
                "200",
                None,
                Some("bash"),
                true,
                &tool_processes,
                &mcp_cfg,
            ),
            CommandClassification::Skip
        ));
    }

    #[test]
    fn tool_process_marker_prevents_nested_consumption_and_is_released_on_exit() {
        let mut cfg = test_config();
        cfg.filter.llm_attribution = LlmAttribution::Fifo;
        let mut trace = TraceState::new(1000);
        trace.agent_process_id = Some("100".to_string());

        // LLM 返回工具名 bash → 后续命令（parent 非 agent 也归因）
        trace.observe_llm_response(
            &action_record(
                "llm.response",
                &[(
                    "llm.response.tool_calls_json",
                    r#"[{"function":{"name":"bash"}}]"#,
                )],
            ),
            16,
        );
        trace.observe_command_invocation(
            &action_record(
                "command.invocation",
                &[("process.id", "200"), ("process.parent.id", "999")],
            ),
            &cfg,
            16,
        );
        assert!(trace.tool_processes.contains("200"));
        assert!(trace.pending_tool_calls.is_empty());

        // 嵌套子进程（parent=200）即使有 pending 名也不消耗
        trace.observe_llm_response(
            &action_record(
                "llm.response",
                &[(
                    "llm.response.tool_calls_json",
                    r#"[{"function":{"name":"read"}}]"#,
                )],
            ),
            16,
        );
        trace.observe_command_invocation(
            &action_record(
                "command.invocation",
                &[("process.id", "201"), ("process.parent.id", "200")],
            ),
            &cfg,
            16,
        );
        assert_eq!(trace.pending_tool_calls.len(), 1);
        assert_eq!(
            trace
                .pending_tool_calls
                .front()
                .map(|call| call.name.as_str()),
            Some("read")
        );

        // 工具进程退出后释放标记
        trace.observe_process_exit(
            &action_record(
                "process.exit",
                &[("process.id", "200"), ("process.exit_code", "2")],
            ),
            &cfg,
            1001,
        );
        assert!(!trace.tool_processes.contains("200"));
    }

    #[test]
    fn hint_matching_ignores_unrelated_commands() {
        // false 必须是独立 token：不能匹配 core.fsmonitor=false
        assert!(contains_hint("/bin/bash -c false", "false"));
        assert!(!contains_hint("git -c core.fsmonitor=false", "false"));
        assert!(contains_hint(
            "/bin/bash -c nonexistent-command-xyz",
            "nonexistent-command-xyz"
        ));
        assert!(contains_hint("/bin/bash -c ls /lll", "ls /lll"));
        assert!(!contains_hint("git rev-parse --show-toplevel", "ls /lll"));
        assert!(!contains_hint("", "false"));
    }

    #[test]
    fn hint_match_beats_fifo_so_git_noise_does_not_steal_names() {
        let cfg = test_config();
        let mut pending: VecDeque<PendingToolCall> = VecDeque::new();
        pending.push_back(PendingToolCall {
            name: "bash".to_string(),
            hint: "nonexistent-command-xyz".to_string(),
            observed_ms: 1000,
        });
        // git 命令行不包含提示 → 不消耗
        assert_eq!(
            take_pending_tool_call(
                &mut pending,
                "git rev-parse --show-toplevel",
                "/usr/bin/git",
                &cfg,
            ),
            None
        );
        assert_eq!(pending.len(), 1);
        // 真正的 bash 工具命令包含提示 → 消耗
        assert_eq!(
            take_pending_tool_call(
                &mut pending,
                "/bin/bash -c nonexistent-command-xyz",
                "/bin/bash",
                &cfg,
            ),
            Some("bash".to_string())
        );
        assert!(pending.is_empty());
    }

    #[test]
    fn repeated_streaming_tool_call_id_is_enqueued_once() {
        let cfg = test_config();
        let mut trace = TraceState::new(1000);
        let json =
            r#"[{"id":"call-a","function":{"arguments_json":{"command":"ls /a"},"name":"bash"}}]"#;

        let mut first = action_record("llm.response", &[("llm.response.tool_calls_json", json)]);
        first.action_id = "response-a-partial".to_string();
        trace.observe_llm_response_at(&first, &cfg, 1000, 16);

        let mut repeated = first.clone();
        repeated.action_id = "response-a-final".to_string();
        trace.observe_llm_response_at(&repeated, &cfg, 1001, 16);

        assert_eq!(trace.pending_tool_calls.len(), 1);
        assert_eq!(trace.seen_tool_calls.len(), 1);
    }

    #[test]
    fn stale_hinted_call_does_not_block_later_deferred_command() {
        let cfg = test_config();
        let mut trace = TraceState::new(1000);
        trace.agent_process_id = Some("100".to_string());

        trace.observe_llm_response_at(
            &action_record(
                "llm.response",
                &[(
                    "llm.response.tool_calls_json",
                    r#"[{"id":"call-a","function":{"arguments_json":{"command":"ls /a"},"name":"bash"}}]"#,
                )],
            ),
            &cfg,
            1000,
            16,
        );
        trace.observe_command_invocation(
            &action_record(
                "command.invocation",
                &[
                    ("process.id", "202"),
                    ("process.parent.id", "100"),
                    ("command.line", "/bin/bash -c ls /b"),
                    ("process.executable", "/bin/bash"),
                ],
            ),
            &cfg,
            16,
        );
        assert!(trace.commands_by_process["202"].deferred_attribution);

        let result = trace.observe_process_exit(
            &action_record(
                "process.exit",
                &[("process.id", "202"), ("process.exit_code", "2")],
            ),
            &cfg,
            1001,
        );
        assert!(!result.record);

        let resolved = trace.observe_llm_response_at(
            &action_record(
                "llm.response",
                &[(
                    "llm.response.tool_calls_json",
                    r#"[{"id":"call-b","function":{"arguments_json":{"command":"ls /b"},"name":"bash"}}]"#,
                )],
            ),
            &cfg,
            1002,
            16,
        );
        assert_eq!(resolved.len(), 1);
        assert!(resolved[0].result.record);
        assert_eq!(resolved[0].result.tool_name, "bash");
        assert_eq!(resolved[0].result.outcome, Outcome::Failure);
    }

    #[test]
    fn sequential_agent_calls_with_late_responses_reach_threshold() {
        let mut cfg = test_config();
        cfg.alert.min_failure_count = 3;
        let mut trace = TraceState::new(1000);
        trace.agent_process_id = Some("100".to_string());
        let mut final_alert = None;

        for (index, path) in ["/a", "/b", "/c"].into_iter().enumerate() {
            let pid = format!("20{index}");
            let command_line = format!("/bin/bash -c ls {path}");
            let command_hint = format!("ls {path}");
            let call_json = format!(
                r#"[{{"id":"call-{index}","function":{{"arguments_json":{{"command":"{command_hint}"}},"name":"bash"}}}}]"#
            );

            if index == 0 {
                trace.observe_llm_response_at(
                    &action_record(
                        "llm.response",
                        &[("llm.response.tool_calls_json", &call_json)],
                    ),
                    &cfg,
                    1000,
                    16,
                );
            }

            let mut command = action_record(
                "command.invocation",
                &[
                    ("process.id", &pid),
                    ("process.parent.id", "100"),
                    ("command.line", &command_line),
                    ("process.executable", "/bin/bash"),
                ],
            );
            command.action_id = format!("command-{index}");
            trace.observe_command_invocation(&command, &cfg, 16);

            let mut exit = action_record(
                "process.exit",
                &[("process.id", &pid), ("process.exit_code", "2")],
            );
            exit.action_id = format!("exit-{index}");
            let immediate = trace.observe_process_exit(&exit, &cfg, 1001 + index as u64);

            let executions = if immediate.record {
                vec![DeferredExecution {
                    result: immediate,
                    exit_action_id: exit.action_id.clone(),
                    observed_ms: 1001 + index as u64,
                }]
            } else {
                let mut response = action_record(
                    "llm.response",
                    &[("llm.response.tool_calls_json", &call_json)],
                );
                response.action_id = format!("response-{index}");
                trace.observe_llm_response_at(&response, &cfg, 1002 + index as u64, 16)
            };

            for execution in executions {
                let result = execution.result;
                final_alert = trace.record_outcome(
                    &result.tool_name,
                    &cfg.map_failure_type(&result.raw_failure_type),
                    &result.exit_status,
                    result.outcome,
                    &execution.exit_action_id,
                    &[&result.command_action_id, &execution.exit_action_id],
                    1002 + index as u64,
                    &cfg,
                    &result.summary,
                    &result.command_line,
                );
            }

            if index == 0 {
                let mut repeated = action_record(
                    "llm.response",
                    &[("llm.response.tool_calls_json", &call_json)],
                );
                repeated.action_id = "response-0-repeated".to_string();
                trace.observe_llm_response_at(&repeated, &cfg, 1003, 16);
            }
        }

        let alert = final_alert.expect("three sequential failures must alert");
        assert_eq!(alert.tool_name, "bash");
        assert_eq!(alert.failure_count, 3);
    }

    #[test]
    fn exec_replacement_does_not_override_monitored_command() {
        let cfg = test_config();
        let mut trace = TraceState::new(1000);
        trace.agent_process_id = Some("100".to_string());
        trace.observe_llm_response(
            &action_record(
                "llm.response",
                &[(
                    "llm.response.tool_calls_json",
                    r#"[{"function":{"arguments_json":{"command":"ls /lll"},"name":"bash"}}]"#,
                )],
            ),
            16,
        );
        // 第一次：bash 工具命令，命中提示
        trace.observe_command_invocation(
            &action_record(
                "command.invocation",
                &[
                    ("process.id", "296"),
                    ("process.parent.id", "100"),
                    ("command.line", "/bin/bash -c ls /lll"),
                    ("process.executable", "/bin/bash"),
                ],
            ),
            &cfg,
            16,
        );
        // 第二次：同一进程 exec 替换为 ls，不能覆盖首条登记
        trace.observe_command_invocation(
            &action_record(
                "command.invocation",
                &[
                    ("process.id", "296"),
                    ("process.parent.id", "100"),
                    ("command.line", "ls /lll"),
                    ("process.executable", "/usr/bin/ls"),
                ],
            ),
            &cfg,
            16,
        );
        let entry = trace
            .commands_by_process
            .get("296")
            .expect("first registration kept");
        assert!(entry.monitored);
        assert_eq!(entry.tool_name, "bash");
        assert!(trace.pending_tool_calls.is_empty());
    }

    #[test]
    fn terminal_trace_state_survives_attribution_grace() {
        let mut trace = TraceState::new(1000);
        trace.terminal_ms = Some(1000);
        let grace_ms = 10_000;
        assert!(2000_u64.saturating_sub(trace.terminal_ms.unwrap()) < grace_ms);
        assert!(!(11_001_u64.saturating_sub(trace.terminal_ms.unwrap()) < grace_ms));
    }

    #[test]
    fn command_before_llm_response_is_attributed_before_exit() {
        let mut cfg = test_config();
        cfg.alert.min_failure_count = 1;
        let mut trace = TraceState::new(1000);
        trace.agent_process_id = Some("100".to_string());

        trace.observe_command_invocation(
            &action_record(
                "command.invocation",
                &[
                    ("process.id", "200"),
                    ("process.parent.id", "100"),
                    ("command.line", "/bin/bash -c ls /late-running"),
                    ("process.executable", "/bin/bash"),
                ],
            ),
            &cfg,
            16,
        );
        assert!(trace.commands_by_process["200"].deferred_attribution);

        let resolved = trace.observe_llm_response_at(
            &action_record(
                "llm.response",
                &[(
                    "llm.response.tool_calls_json",
                    r#"[{"function":{"arguments_json":{"command":"ls /late-running"},"name":"bash"}}]"#,
                )],
            ),
            &cfg,
            1001,
            16,
        );
        assert!(resolved.is_empty());
        let entry = &trace.commands_by_process["200"];
        assert!(entry.llm_attributed);
        assert!(!entry.deferred_attribution);
        assert_eq!(entry.tool_name, "bash");

        let result = trace.observe_process_exit(
            &action_record(
                "process.exit",
                &[("process.id", "200"), ("process.exit_code", "2")],
            ),
            &cfg,
            1002,
        );
        assert!(result.record);
        assert_eq!(result.tool_name, "bash");
        assert_eq!(result.outcome, Outcome::Failure);
    }

    #[test]
    fn in_progress_exit_waits_only_without_terminal_evidence() {
        let mut cfg = test_config();
        cfg.alert.min_failure_count = 1;
        let mut trace = TraceState::new(1000);
        trace.agent_process_id = Some("100".to_string());
        trace.observe_llm_response_at(
            &action_record(
                "llm.response",
                &[(
                    "llm.response.tool_calls_json",
                    r#"[{"function":{"arguments_json":{"command":"ls /progress"},"name":"bash"}}]"#,
                )],
            ),
            &cfg,
            1000,
            16,
        );
        trace.observe_command_invocation(
            &action_record(
                "command.invocation",
                &[
                    ("process.id", "200"),
                    ("process.parent.id", "100"),
                    ("command.line", "/bin/bash -c ls /progress"),
                    ("process.executable", "/bin/bash"),
                ],
            ),
            &cfg,
            16,
        );

        let mut pending_exit = action_record("process.exit", &[("process.id", "200")]);
        pending_exit.status = "in_progress".to_string();
        let pending = trace.observe_process_exit(&pending_exit, &cfg, 1001);
        assert!(pending.matched);
        assert!(!pending.record);
        assert!(trace.commands_by_process.contains_key("200"));

        let mut final_exit = action_record(
            "process.exit",
            &[("process.id", "200"), ("process.exit_code", "2")],
        );
        final_exit.status = "in_progress".to_string();
        let result = trace.observe_process_exit(&final_exit, &cfg, 1002);
        assert!(result.matched);
        assert!(result.record);
        assert_eq!(result.tool_name, "bash");
        assert_eq!(result.outcome, Outcome::Failure);
        assert!(!trace.commands_by_process.contains_key("200"));
    }

    #[test]
    fn completed_commands_are_replayed_after_streaming_llm_response() {
        let mut cfg = test_config();
        cfg.alert.min_failure_count = 2;
        let mut trace = TraceState::new(1000);
        trace.agent_process_id = Some("100".to_string());

        // OpenCode 内部 git 命令也是 agent 子进程，但不能抢走带参数提示的工具调用。
        for (pid, line, code, command_id, exit_id) in [
            (
                "200",
                "git rev-parse --show-toplevel",
                "0",
                "git-command",
                "git-exit",
            ),
            (
                "201",
                "/bin/bash -c missing-late-a",
                "127",
                "command-a",
                "exit-a",
            ),
            (
                "202",
                "/bin/bash -c missing-late-b",
                "127",
                "command-b",
                "exit-b",
            ),
        ] {
            let mut command = action_record(
                "command.invocation",
                &[
                    ("process.id", pid),
                    ("process.parent.id", "100"),
                    ("command.line", line),
                    ("process.executable", "/bin/bash"),
                ],
            );
            command.action_id = command_id.to_string();
            trace.observe_command_invocation(&command, &cfg, 16);
            let mut exit = action_record(
                "process.exit",
                &[("process.id", pid), ("process.exit_code", code)],
            );
            exit.action_id = exit_id.to_string();
            let result = trace.observe_process_exit(&exit, &cfg, 1001);
            assert!(!result.record, "unattributed result must be deferred");
        }
        assert_eq!(trace.completed_unattributed.len(), 3);

        let resolved = trace.observe_llm_response_at(
            &action_record(
                "llm.response",
                &[(
                    "llm.response.tool_calls_json",
                    r#"[
                        {"function":{"arguments_json":{"command":"missing-late-a"},"name":"bash"}},
                        {"function":{"arguments_json":{"command":"missing-late-b"},"name":"bash"}}
                    ]"#,
                )],
            ),
            &cfg,
            1002,
            16,
        );
        assert_eq!(resolved.len(), 2);
        assert_eq!(
            trace.completed_unattributed.len(),
            1,
            "git remains unmatched"
        );

        let mut alert = None;
        for execution in resolved {
            let result = execution.result;
            let failure_type = cfg.map_failure_type(&result.raw_failure_type);
            alert = trace.record_outcome(
                &result.tool_name,
                &failure_type,
                &result.exit_status,
                result.outcome,
                &execution.exit_action_id,
                &[&result.command_action_id, &execution.exit_action_id],
                1002,
                &cfg,
                &result.summary,
                &result.command_line,
            );
        }
        let alert = alert.expect("two replayed failures must alert");
        assert_eq!(alert.tool_name, "bash");
        assert_eq!(alert.failure_count, 2);
        assert_eq!(alert.total_count_for_test(), 2);
    }

    #[test]
    fn deferred_execution_expires_after_attribution_grace() {
        let mut cfg = test_config();
        cfg.resources.attribution_grace_seconds = 1;
        let mut trace = TraceState::new(1000);
        trace.agent_process_id = Some("100".to_string());
        trace.observe_command_invocation(
            &action_record(
                "command.invocation",
                &[
                    ("process.id", "200"),
                    ("process.parent.id", "100"),
                    ("command.line", "/bin/bash -c missing-expired"),
                    ("process.executable", "/bin/bash"),
                ],
            ),
            &cfg,
            16,
        );
        trace.observe_process_exit(
            &action_record(
                "process.exit",
                &[("process.id", "200"), ("process.exit_code", "127")],
            ),
            &cfg,
            1000,
        );
        let resolved = trace.observe_llm_response_at(
            &action_record(
                "llm.response",
                &[(
                    "llm.response.tool_calls_json",
                    r#"[{"function":{"arguments_json":{"command":"missing-expired"},"name":"bash"}}]"#,
                )],
            ),
            &cfg,
            2001,
            16,
        );
        assert!(resolved.is_empty());
        assert!(trace.completed_unattributed.is_empty());
    }

    #[test]
    fn heterogeneous_exit_codes_aggregate_under_one_tool() {
        let mut trace = TraceState::new(1000);
        let cfg = test_config();
        // opencode 场景：同一工具三次失败、三个不同退出码（127/2/1）
        let mut alert = None;
        for (index, (action_id, code)) in [("f0", "127"), ("f1", "2"), ("f2", "1")]
            .into_iter()
            .enumerate()
        {
            alert = trace.record_outcome(
                "bash",
                "runtime_error",
                code,
                Outcome::Failure,
                action_id,
                &[action_id],
                1000 + index as u64,
                &cfg,
                "exit code",
                "",
            );
        }
        let alert = alert.expect("3 heterogeneous failures on one tool must alert");
        assert_eq!(alert.failure_count, 3);
        assert_eq!(alert.failure_type, "runtime_error");
        assert_eq!(alert.failure_breakdown.len(), 3);
    }

    #[test]
    fn shell_matrix_parent_scope_any_counts_ls_failures() {
        // 复现回归用例的 shell 矩阵（含真实拓扑）：
        // 根 bash 对 -c 的最后一个简单命令直接 exec 成 ls（同一进程 1），
        // 无 LLM、parent_scope=any、三条 ls 失败（退出码 2）应全部归一到 "ls"。
        let mut cfg = test_config();
        cfg.filter.tool_scope = ToolScope::AgentChildren;
        cfg.filter.parent_scope = ParentScope::Any;
        let mut trace = TraceState::new(1000);
        trace.observe_command_invocation(
            &action_record(
                "command.invocation",
                &[
                    ("process.id", "1"),
                    ("process.parent.id", "0"),
                    ("command.line", "bash -c ls /actrail-missing-frequent-a"),
                    ("process.executable", "/bin/bash"),
                ],
            ),
            &cfg,
            16,
        );
        // 三次 ls：pid 2、pid 3 为独立子进程，第三次 bash exec 替换为 ls（仍为 pid 1）
        let commands = [
            ("2", "1", "ls /actrail-missing-frequent-a"),
            ("3", "1", "ls /actrail-missing-frequent-b"),
            ("1", "0", "ls /actrail-missing-frequent-c"),
        ];
        let mut alert = None;
        for (index, (pid, parent, path)) in commands.into_iter().enumerate() {
            trace.observe_command_invocation(
                &action_record(
                    "command.invocation",
                    &[
                        ("process.id", pid),
                        ("process.parent.id", parent),
                        ("command.line", path),
                        ("process.executable", "/usr/bin/ls"),
                    ],
                ),
                &cfg,
                16,
            );
            let exit_action_id = format!("exit-{pid}");
            let result = trace.observe_process_exit(
                &action_record(
                    "process.exit",
                    &[("process.id", pid), ("process.exit_code", "2")],
                ),
                &cfg,
                1000 + index as u64,
            );
            assert!(result.record, "ls failure must be recorded");
            assert_eq!(result.tool_name, "ls");
            assert_eq!(result.outcome, Outcome::Failure);
            let failure_type = cfg.map_failure_type(&result.raw_failure_type);
            let evidence_ids = [result.command_action_id.as_str(), exit_action_id.as_str()];
            alert = trace.record_outcome(
                &result.tool_name,
                &failure_type,
                &result.exit_status,
                result.outcome,
                &exit_action_id,
                &evidence_ids,
                1000 + index as u64,
                &cfg,
                &result.summary,
                &result.command_line,
            );
        }
        let alert = alert.expect("3 ls failures under parent_scope=any must alert");
        assert_eq!(alert.tool_name, "ls");
        assert_eq!(alert.failure_count, 3);
    }

    #[test]
    fn shell_root_exec_replacement_rebinds_failure_to_final_executable() {
        let mut cfg = test_config();
        cfg.filter.tool_scope = ToolScope::AgentChildren;
        cfg.filter.parent_scope = ParentScope::Any;
        let mut trace = TraceState::new(1000);
        // 根 bash（pid 1）先登记，随后 exec 替换为 ls（pid 1）
        trace.observe_command_invocation(
            &action_record(
                "command.invocation",
                &[
                    ("process.id", "1"),
                    ("process.parent.id", "0"),
                    ("command.line", "bash -c ls /actrail-missing-frequent-c"),
                    ("process.executable", "/bin/bash"),
                ],
            ),
            &cfg,
            16,
        );
        trace.observe_command_invocation(
            &action_record(
                "command.invocation",
                &[
                    ("process.id", "1"),
                    ("process.parent.id", "0"),
                    ("command.line", "ls /actrail-missing-frequent-c"),
                    ("process.executable", "/usr/bin/ls"),
                ],
            ),
            &cfg,
            16,
        );
        let entry = trace.commands_by_process.get("1").expect("rebound entry");
        assert!(entry.monitored);
        assert_eq!(entry.tool_name, "ls");
        assert!(!entry.llm_attributed);
    }

    #[test]
    fn llm_attributed_exec_replacement_keeps_tool_name() {
        // opencode 场景：bash 工具（LLM 归因）exec 成 ls 后，工具名保持 bash
        let mut cfg = test_config();
        cfg.filter.llm_attribution = LlmAttribution::Fifo;
        let mut trace = TraceState::new(1000);
        trace.agent_process_id = Some("100".to_string());
        trace.observe_llm_response(
            &action_record(
                "llm.response",
                &[(
                    "llm.response.tool_calls_json",
                    r#"[{"function":{"arguments_json":{"command":"ls /lll"},"name":"bash"}}]"#,
                )],
            ),
            16,
        );
        trace.observe_command_invocation(
            &action_record(
                "command.invocation",
                &[
                    ("process.id", "296"),
                    ("process.parent.id", "100"),
                    ("command.line", "/bin/bash -c ls /lll"),
                    ("process.executable", "/bin/bash"),
                ],
            ),
            &cfg,
            16,
        );
        trace.observe_command_invocation(
            &action_record(
                "command.invocation",
                &[
                    ("process.id", "296"),
                    ("process.parent.id", "100"),
                    ("command.line", "ls /lll"),
                    ("process.executable", "/usr/bin/ls"),
                ],
            ),
            &cfg,
            16,
        );
        let entry = trace.commands_by_process.get("296").expect("kept entry");
        assert!(entry.monitored);
        assert_eq!(entry.tool_name, "bash");
        assert!(entry.llm_attributed);
    }
}

impl AlertData {
    #[cfg(test)]
    fn total_count_for_test(&self) -> u64 {
        self.failure_count + self.success_count
    }
}
