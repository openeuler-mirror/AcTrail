#![no_std]

extern crate alloc;

use alloc::collections::{BTreeMap, VecDeque};
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt::Write;

use alloc::alloc::{Layout, alloc, realloc};

#[global_allocator]
static ALLOCATOR: wee_alloc::WeeAlloc = wee_alloc::WeeAlloc::INIT;

wit_bindgen::generate!({
    path: "../../../../crates/core/plugin_system/wit",
    world: "observation-plugin",
});

use exports::actrail::plugin::observation_consumer::{Guest, ObservationBatch, ObservationReport};

// ============================================================================
// 常量
// ============================================================================

/// 默认连续失败阈值
const DEFAULT_THRESHOLD: u32 = 3;
/// 默认冷却时间（秒）
const DEFAULT_COOLDOWN_SECS: u64 = 60;
/// 默认状态 TTL（秒）
const DEFAULT_STATE_TTL_SECS: u64 = 300;
/// 默认脱敏模式
const DEFAULT_DESENSITIZATION: &str = "summary_only";
/// 默认工具名格式：bare（仅工具名）或 full（含参数的完整命令行）
const DEFAULT_TOOL_NAME_FORMAT: &str = "bare";
/// 默认：策略拒绝是否计入失败
const DEFAULT_POLICY_DENIED_COUNTS_AS_FAILURE: bool = true;
/// 默认：process.exit 状态为 unknown（宿主在退出码为 0 时不上报 exit_code）时视为成功
const DEFAULT_UNKNOWN_STATUS_COUNTS_AS_SUCCESS: bool = true;
/// 宿主导出时统一注入的进程标识属性（见 wire.rs observation_attributes）
const PROCESS_ID_ATTR: &str = "process.id";
/// process.exit 动作上的退出码属性（仅非 0 时存在）
const PROCESS_EXIT_CODE_ATTR: &str = "process.exit_code";
/// process.exit 动作上的失败摘要属性
const PROCESS_FAILURE_SUMMARY_ATTR: &str = "process.failure.summary";
/// 工具名属性（host 可能回填）
const COMMAND_TOOL_NAME_ATTR: &str = "command.tool.name";
/// 可执行文件属性
const PROCESS_EXECUTABLE_ATTR: &str = "process.executable";
/// 命令行属性
const COMMAND_LINE_ATTR: &str = "command.line";

// ============================================================================
// 状态机
// ============================================================================

/// 单个工具的连续失败状态
struct ToolFailureState {
    /// 连续失败次数
    count: u32,
    /// 第一个失败 action_id
    first_action_id: String,
    /// 最后一个失败 action_id
    last_action_id: String,
    /// 所有失败证据 action_id 列表
    evidence_action_ids: Vec<String>,
    /// 上次告警时间（epoch 秒），用于冷却判断
    last_alert_secs: u64,
    /// 最后活跃时间（epoch 秒），用于 TTL 回收
    last_active_secs: u64,
}

impl ToolFailureState {
    fn new(epoch_secs: u64) -> Self {
        Self {
            count: 0,
            first_action_id: String::new(),
            last_action_id: String::new(),
            evidence_action_ids: Vec::new(),
            last_alert_secs: 0,
            last_active_secs: epoch_secs,
        }
    }

    /// 记录一次失败。`command_action_id` 是对应 command.invocation 的 action_id，
    /// `exit_action_id` 是对应 process.exit 的 action_id（唯一性去重依据）。
    /// 返回 false 表示该 exit 事件此前已处理过（跨 batch 重发）。
    fn record_failure(
        &mut self,
        command_action_id: &str,
        exit_action_id: &str,
        epoch_secs: u64,
    ) -> bool {
        if self
            .evidence_action_ids
            .iter()
            .any(|id| id == exit_action_id)
        {
            self.last_active_secs = epoch_secs;
            return false;
        }
        if self.count == 0 {
            self.first_action_id = command_action_id.to_string();
        }
        self.count += 1;
        self.last_action_id = exit_action_id.to_string();
        if !self
            .evidence_action_ids
            .iter()
            .any(|id| id == command_action_id)
        {
            self.evidence_action_ids.push(command_action_id.to_string());
        }
        self.evidence_action_ids.push(exit_action_id.to_string());
        self.last_active_secs = epoch_secs;
        true
    }

    fn record_success(&mut self, epoch_secs: u64) {
        self.count = 0;
        self.evidence_action_ids.clear();
        self.first_action_id.clear();
        self.last_action_id.clear();
        self.last_active_secs = epoch_secs;
    }
}

type StateKey = (String, String);

/// 一次 command.invocation 的登记信息，等待对应的 process.exit 回填成败。
#[derive(Clone)]
struct CommandEntry {
    action_id: String,
    bare_tool_name: String,
    effective_tool_name: String,
    tool_args: String,
}

struct PluginState {
    /// 按 (trace_id, tool_name) 维护的状态
    states: BTreeMap<StateKey, ToolFailureState>,
    /// 按 trace 维护待决命令队列（FIFO 兜底关联，process.id 缺失/漏采时使用）
    pending: BTreeMap<String, VecDeque<CommandEntry>>,
    /// 按 (trace_id, process.id) 精确关联 command.invocation -> 待决命令
    commands_by_process: BTreeMap<(String, String), CommandEntry>,
    /// 配置
    threshold: u32,
    cooldown_secs: u64,
    state_ttl_secs: u64,
    desensitization: String,
    monitored_tools: Vec<String>,
    ignored_tools: Vec<String>,
    policy_denied_counts_as_failure: bool,
    tool_name_format: String,
    unknown_status_counts_as_success: bool,
    /// 单调递增的批时钟（用于冷却/TTL 的近似时间；告警 timestamp 由宿主落库时覆盖）
    clock: u64,
}

impl PluginState {
    fn new() -> Self {
        Self {
            states: BTreeMap::new(),
            pending: BTreeMap::new(),
            commands_by_process: BTreeMap::new(),
            threshold: DEFAULT_THRESHOLD,
            cooldown_secs: DEFAULT_COOLDOWN_SECS,
            state_ttl_secs: DEFAULT_STATE_TTL_SECS,
            desensitization: DEFAULT_DESENSITIZATION.to_string(),
            monitored_tools: Vec::new(),
            ignored_tools: Vec::new(),
            policy_denied_counts_as_failure: DEFAULT_POLICY_DENIED_COUNTS_AS_FAILURE,
            tool_name_format: DEFAULT_TOOL_NAME_FORMAT.to_string(),
            unknown_status_counts_as_success: DEFAULT_UNKNOWN_STATUS_COUNTS_AS_SUCCESS,
            clock: 0,
        }
    }

    /// 加载配置（从 TOML 字符串解析）
    fn load_config(&mut self, config_str: &str) {
        if let Some(val) = parse_toml_u32(config_str, "alert", "consecutive_failure_threshold") {
            self.threshold = val;
        }
        if let Some(val) = parse_toml_u64(config_str, "alert", "cooldown_seconds") {
            self.cooldown_secs = val;
        }
        if let Some(val) = parse_toml_u64(config_str, "alert.behavior", "state_ttl_seconds") {
            self.state_ttl_secs = val;
        }
        if let Some(val) = parse_toml_bool(
            config_str,
            "alert.behavior",
            "policy_denied_counts_as_failure",
        ) {
            self.policy_denied_counts_as_failure = val;
        }
        if let Some(val) = parse_toml_bool(
            config_str,
            "alert.behavior",
            "unknown_status_counts_as_success",
        ) {
            self.unknown_status_counts_as_success = val;
        }
        if let Some(val) = parse_toml_string(config_str, "alert", "desensitization") {
            self.desensitization = val;
        }
        if let Some(val) = parse_toml_string_list(config_str, "alert.filter", "monitored_tools") {
            self.monitored_tools = val;
        }
        if let Some(val) = parse_toml_string_list(config_str, "alert.filter", "ignored_tools") {
            self.ignored_tools = val;
        }
        if let Some(val) = parse_toml_string(config_str, "alert", "tool_name_format") {
            if val == "bare" || val == "full" {
                self.tool_name_format = val;
            }
        }
    }

    /// 判断是否应监控该工具
    fn should_monitor(&self, tool_name: &str) -> bool {
        if self.ignored_tools.iter().any(|t| t == tool_name) {
            return false;
        }
        if self.monitored_tools.is_empty() {
            return true;
        }
        self.monitored_tools.iter().any(|t| t == tool_name)
    }

    /// 判断是否为策略拒绝（简单启发式）
    fn is_policy_denied(failure_summary: &str) -> bool {
        let lower = failure_summary.to_lowercase();
        lower.contains("policy") && (lower.contains("denied") || lower.contains("reject"))
    }

    /// 登记一次 command.invocation（只登记，不判成败）。
    /// 成败统一由对应的 process.exit 事件决定。
    fn register_command_invocation(
        &mut self,
        trace_id: &str,
        action_id: &str,
        bare_tool_name: &str,
        effective_tool_name: &str,
        tool_args: &str,
        process_id: &str,
    ) {
        let entry = CommandEntry {
            action_id: action_id.to_string(),
            bare_tool_name: bare_tool_name.to_string(),
            effective_tool_name: effective_tool_name.to_string(),
            tool_args: tool_args.to_string(),
        };
        self.pending
            .entry(trace_id.to_string())
            .or_default()
            .push_back(entry.clone());
        if !process_id.is_empty() {
            self.commands_by_process.insert(
                (trace_id.to_string(), process_id.to_string()),
                entry,
            );
        }
    }

    /// 处理一次 process.exit：精确关联到对应命令（process.id），
    /// 更新 (trace, tool) 的连续成败计数，达到阈值时返回告警 JSON。
    fn process_exit(
        &mut self,
        trace_id: &str,
        action_id: &str,
        status: &str,
        exit_code: &str,
        failure_summary: &str,
        process_id: &str,
    ) -> Option<String> {
        // 1) 关联命令：优先按 (trace, process.id) 精确匹配；缺失时用 FIFO 兜底
        let entry = if !process_id.is_empty() {
            self.commands_by_process
                .remove(&(trace_id.to_string(), process_id.to_string()))
        } else {
            None
        }
        .or_else(|| {
            self.pending
                .get_mut(trace_id)
                .and_then(|queue| queue.pop_front())
        });
        let entry = entry?;
        self.remove_pending_entry(trace_id, &entry.action_id);

        // 2) 过滤与状态判定
        if !self.should_monitor(&entry.bare_tool_name) {
            return None;
        }
        if status == "in_progress" {
            return None;
        }

        let is_failure = status == "error" || (!exit_code.is_empty() && exit_code != "0");
        let is_success = status == "success"
            || (status == "unknown" && self.unknown_status_counts_as_success);

        let key: StateKey = (trace_id.to_string(), entry.effective_tool_name.clone());
        if is_success {
            // 成功：重置计数器
            if let Some(state) = self.states.get_mut(&key) {
                state.record_success(self.clock);
            }
            return None;
        }
        if !is_failure {
            // unknown 且配置为不计成功：忽略（不清零也不计数）
            return None;
        }
        // 策略拒绝检查：如果配置为不计入失败，则跳过
        if Self::is_policy_denied(failure_summary) && !self.policy_denied_counts_as_failure {
            return None;
        }

        // 3) 更新失败状态（跨 batch 重发的 exit 事件按 action_id 去重）
        let state = self
            .states
            .entry(key.clone())
            .or_insert_with(|| ToolFailureState::new(self.clock));
        if !state.record_failure(&entry.action_id, action_id, self.clock) {
            return None;
        }

        // 4) 阈值 + 冷却检查
        if state.count < self.threshold {
            return None;
        }
        if self.cooldown_secs > 0
            && state.last_alert_secs > 0
            && self.clock - state.last_alert_secs < self.cooldown_secs
        {
            return None; // 冷却中，不重复告警
        }
        state.last_alert_secs = self.clock;

        // 提取 state 字段，避免同时持有可变借用和不可变借用
        let count = state.count;
        let first_action_id = state.first_action_id.clone();
        let last_action_id = state.last_action_id.clone();
        let evidence_action_ids = state.evidence_action_ids.clone();
        let threshold = self.threshold;
        let desensitization = self.desensitization.clone();
        let summary = if failure_summary.is_empty() && !exit_code.is_empty() {
            alloc::format!("exit code {exit_code}")
        } else {
            failure_summary.to_string()
        };

        Some(Self::build_alert_static(
            trace_id,
            &entry.effective_tool_name,
            &entry.tool_args,
            count,
            threshold,
            &desensitization,
            &first_action_id,
            &last_action_id,
            &evidence_action_ids,
            &summary,
            self.clock,
        ))
    }

    /// 从 per-trace FIFO 中移除已关联的命令（避免残留错配）。
    fn remove_pending_entry(&mut self, trace_id: &str, action_id: &str) {
        let remove_trace = if let Some(queue) = self.pending.get_mut(trace_id) {
            queue.retain(|entry| entry.action_id != action_id);
            queue.is_empty()
        } else {
            false
        };
        if remove_trace {
            self.pending.remove(trace_id);
        }
    }

    /// 构建告警 JSON（静态方法，不借用 self）
    fn build_alert_static(
        _trace_id: &str,
        tool_name: &str,
        tool_args: &str,
        count: u32,
        threshold: u32,
        desensitization: &str,
        first_action_id: &str,
        last_action_id: &str,
        evidence_action_ids: &[String],
        failure_summary: &str,
        epoch_secs: u64,
    ) -> String {
        let summary = escape_json(failure_summary);
        let timestamp = epoch_to_iso8601(epoch_secs);
        // summary_only 模式下不上报命令行明文
        let safe_args = if desensitization == "summary_only" {
            ""
        } else {
            tool_args
        };

        let mut alert = String::new();
        let _ = write!(
            alert,
            r#"{{"alert_type":"consecutive_failure","timestamp":"{}","tool_name":"{}","tool_args":"{}","consecutive_failures":{},"threshold":{},"failure_summary":"{}","failure_sequence":{{"first_action_id":"{}","last_action_id":"{}"}},"evidence_action_ids":["#,
            timestamp,
            escape_json(tool_name),
            escape_json(safe_args),
            count,
            threshold,
            summary,
            escape_json(first_action_id),
            escape_json(last_action_id),
        );

        for (i, action_id) in evidence_action_ids.iter().enumerate() {
            if i > 0 {
                let _ = write!(alert, ",");
            }
            let _ = write!(alert, "\"{}\"", escape_json(action_id));
        }

        let _ = write!(alert, "]}}");
        alert
    }

    /// 清理过期状态（TTL 回收）
    fn cleanup_expired_states(&mut self) {
        if self.state_ttl_secs == 0 {
            return;
        }

        let ttl = self.state_ttl_secs;
        let now = self.clock;
        self.states
            .retain(|_, state| now - state.last_active_secs < ttl);
    }
}

// ============================================================================
// 简易 TOML 解析器（no_std 环境）
// ============================================================================

/// 从 TOML 字符串中解析 u32 值
/// 支持的格式：`key = value` 或 `key = "value"`（自动转换）
fn parse_toml_u32(config: &str, section: &str, key: &str) -> Option<u32> {
    let raw = find_toml_value(config, section, key)?;
    // 尝试直接解析
    if let Ok(v) = raw.parse::<u32>() {
        return Some(v);
    }
    // 尝试解析带引号的值
    let trimmed = raw.trim_matches('"').trim_matches('\'');
    trimmed.parse::<u32>().ok()
}

fn parse_toml_u64(config: &str, section: &str, key: &str) -> Option<u64> {
    let raw = find_toml_value(config, section, key)?;
    if let Ok(v) = raw.parse::<u64>() {
        return Some(v);
    }
    let trimmed = raw.trim_matches('"').trim_matches('\'');
    trimmed.parse::<u64>().ok()
}

fn parse_toml_string(config: &str, section: &str, key: &str) -> Option<String> {
    let raw = find_toml_value(config, section, key)?;
    Some(raw.trim_matches('"').trim_matches('\'').to_string())
}

fn parse_toml_bool(config: &str, section: &str, key: &str) -> Option<bool> {
    let raw = find_toml_value(config, section, key)?;
    match raw.trim().trim_matches('"').trim_matches('\'') {
        "true" | "yes" | "1" => Some(true),
        "false" | "no" | "0" => Some(false),
        _ => None,
    }
}

fn parse_toml_string_list(config: &str, section: &str, key: &str) -> Option<Vec<String>> {
    let raw = find_toml_value(config, section, key)?;
    let raw = raw.trim();
    if !raw.starts_with('[') || !raw.ends_with(']') {
        return None;
    }
    let inner = &raw[1..raw.len() - 1];
    let items: Vec<String> = inner
        .split(',')
        .map(|s| s.trim().trim_matches('"').trim_matches('\'').to_string())
        .filter(|s| !s.is_empty())
        .collect();
    Some(items)
}

/// 在 TOML 字符串中查找 section.key 对应的值
fn find_toml_value<'a>(config: &'a str, section: &str, key: &str) -> Option<&'a str> {
    let mut in_target_section = false;
    let section_header = alloc::format!("[{}]", section);

    for line in config.lines() {
        let trimmed = line.trim();

        // 跳过空行和注释
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        // 检查 section header
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_target_section = trimmed == section_header;
            continue;
        }

        if !in_target_section {
            continue;
        }

        // 解析 key = value
        if let Some(eq_pos) = trimmed.find('=') {
            let k = trimmed[..eq_pos].trim();
            let v = trimmed[eq_pos + 1..].trim();
            if k == key {
                return Some(v);
            }
        }
    }

    None
}

// ============================================================================
// JSON 工具函数
// ============================================================================

fn escape_json(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => result.push_str("\\\""),
            '\\' => result.push_str("\\\\"),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(result, "\\u{:04x}", c as u32);
            }
            c => result.push(c),
        }
    }
    result
}

/// 将 epoch 秒转换为 ISO 8601 UTC 字符串（YYYY-MM-DDTHH:MM:SSZ）
fn epoch_to_iso8601(epoch_secs: u64) -> String {
    const DAYS_IN_MONTH: [u32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

    let secs = epoch_secs as u32;
    let total_days = secs / 86400;
    let remaining_secs = secs % 86400;
    let hours = remaining_secs / 3600;
    let minutes = (remaining_secs % 3600) / 60;
    let seconds = remaining_secs % 60;

    // 从 1970-01-01 开始计算年月日
    let mut days_left = total_days;

    // 估算年份（粗略，再精确修正）
    let mut year = 1970u32;
    loop {
        let days_in_year = if is_leap_year(year) { 366 } else { 365 };
        if days_left < days_in_year {
            break;
        }
        days_left -= days_in_year;
        year += 1;
    }

    // 计算月份和日期
    let mut month = 1u32;
    for m in 0..12u32 {
        month = m + 1; // m=0 → month=1 (January)
        let dim = if m == 1 && is_leap_year(year) {
            29
        } else {
            DAYS_IN_MONTH[m as usize]
        };
        if days_left < dim {
            break;
        }
        days_left -= dim;
    }
    // days_left 现在是日期-1
    let day = days_left + 1;

    let mut buf = String::with_capacity(20);
    let _ = write!(
        buf,
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, month, day, hours, minutes, seconds
    );
    buf
}

fn is_leap_year(year: u32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

// ============================================================================
// 全局状态
// ============================================================================

use core::cell::{RefCell, UnsafeCell};

/// 提供 `memcmp` 符号（no_std WASM 缺失）
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memcmp(s1: *const u8, s2: *const u8, n: usize) -> i32 {
    unsafe {
        let a = core::slice::from_raw_parts(s1, n);
        let b = core::slice::from_raw_parts(s2, n);
        for i in 0..n {
            if a[i] != b[i] {
                return (a[i] as i32) - (b[i] as i32);
            }
        }
        0
    }
}

/// 全局插件状态（单线程 WASM，无需 Mutex）
struct SyncCell<T>(T);
unsafe impl<T> Sync for SyncCell<T> {}

static STATE: SyncCell<UnsafeCell<Option<RefCell<PluginState>>>> = SyncCell(UnsafeCell::new(None));

fn state() -> &'static RefCell<PluginState> {
    unsafe {
        let state_opt = &mut *STATE.0.get();
        if state_opt.is_none() {
            *state_opt = Some(RefCell::new(PluginState::new()));
        }
        state_opt.as_ref().unwrap()
    }
}

// ============================================================================
// WIT Component 实现
// ============================================================================

struct Component;

impl Guest for Component {
    fn consume(batch: ObservationBatch) -> Result<ObservationReport, String> {
        wit_bindgen::rt::maybe_link_cabi_realloc();

        let state = state();
        let mut state = state.borrow_mut();

        // 尝试加载配置（首次调用时）
        // 通过 read-config 读取配置
        if let Ok(config_str) = read_config_full() {
            state.load_config(&config_str);
        }

        // 单调递增的批时钟：用于冷却 / TTL 的近似时间
        state.clock = state.clock.wrapping_add(1);

        // 遍历 semantic_actions，遇一条处理一条
        let mut observed: u64 = 0;
        for action in &batch.semantic_actions {
            let process_id = find_attr(&action.attributes, PROCESS_ID_ATTR).unwrap_or("");
            if action.kind == "command.invocation" {
                // 只登记，成败由 process.exit 决定
                let bare_tool_name = extract_bare_tool_name(action);
                let tool_args = find_attr(&action.attributes, COMMAND_LINE_ATTR).unwrap_or("");
                let effective_tool_name =
                    if state.tool_name_format == "full" && !tool_args.is_empty() {
                        tool_args
                    } else {
                        bare_tool_name
                    };
                state.register_command_invocation(
                    &action.trace_id,
                    &action.action_id,
                    bare_tool_name,
                    effective_tool_name,
                    tool_args,
                    process_id,
                );
                observed += 1;
            } else if action.kind == "process.exit" {
                let exit_code =
                    find_attr(&action.attributes, PROCESS_EXIT_CODE_ATTR).unwrap_or("");
                let failure_summary =
                    find_attr(&action.attributes, PROCESS_FAILURE_SUMMARY_ATTR).unwrap_or("");
                if let Some(alert_json) = state.process_exit(
                    &action.trace_id,
                    &action.action_id,
                    &action.status,
                    exit_code,
                    failure_summary,
                    process_id,
                ) {
                    submit_alert(&action.trace_id, alert_json);
                }
                observed += 1;
            }
        }

        // 清理过期状态
        state.cleanup_expired_states();

        Ok(ObservationReport {
            observed_records: observed,
            dropped_records: 0,
        })
    }
}

// ============================================================================
// 工具函数
// ============================================================================

/// 查找动作属性值
fn find_attr<'a>(
    attributes: &'a [actrail::plugin::types::AttributePair],
    key: &str,
) -> Option<&'a str> {
    attributes
        .iter()
        .find(|attr| attr.key == key)
        .map(|attr| attr.value.as_str())
}

/// 提取工具名：优先 command.tool.name，fallback 到 process.executable 文件名，
/// 再 fallback 到 command.line 首词
fn extract_bare_tool_name(action: &actrail::plugin::types::SemanticActionRecord) -> &str {
    find_attr(&action.attributes, COMMAND_TOOL_NAME_ATTR)
        .or_else(|| {
            find_attr(&action.attributes, PROCESS_EXECUTABLE_ATTR)
                .and_then(|exec| exec.rsplit('/').next().filter(|s| !s.is_empty()))
        })
        .or_else(|| {
            find_attr(&action.attributes, COMMAND_LINE_ATTR)
                .and_then(|line| line.split_whitespace().next().filter(|s| !s.is_empty()))
        })
        .unwrap_or("unknown")
}

/// 通过 alert-write 接口提交告警到 daemon
fn submit_alert(trace_id: &str, alert_json: String) {
    if let Ok(trace_ctx) = actrail::plugin::observation_context_read::trace_context_get() {
        let alert_token = trace_ctx.alert_token.unwrap_or_default();
        let draft = actrail::plugin::types::AlertDraft {
            definition_key: "consecutive-failure".to_string(),
            payload_json: alert_json,
            deduplication_key: None,
        };
        let request = actrail::plugin::types::AlertWriteRequest {
            trace_id: trace_id.to_string(),
            alert_token,
            draft,
        };
        let _ = actrail::plugin::alert_write::submit(&request);
    }
}

/// 通过 read-config 读取完整配置
fn read_config_full() -> Result<String, String> {
    let mut config = String::new();
    let mut offset: u64 = 0;
    let max_bytes: u64 = 4096;

    loop {
        let chunk = actrail::plugin::host::read_config(offset, max_bytes);
        let bytes = chunk.bytes;
        if bytes.is_empty() {
            break;
        }
        // 将 bytes 转换为字符串
        for &b in &bytes {
            config.push(b as char);
        }
        offset += bytes.len() as u64;
        if chunk.truncated {
            break;
        }
    }

    if config.is_empty() {
        Err("empty config".to_string())
    } else {
        Ok(config)
    }
}

export!(Component);

// ============================================================================
// cabi_realloc
// ============================================================================

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cabi_realloc(
    old_ptr: *mut u8,
    old_len: usize,
    align: usize,
    new_len: usize,
) -> *mut u8 {
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

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {}
}
