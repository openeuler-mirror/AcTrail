#![no_std]

extern crate alloc;

use alloc::collections::BTreeMap;
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

    fn record_failure(&mut self, action_id: String, epoch_secs: u64) {
        if self.evidence_action_ids.contains(&action_id) {
            self.last_active_secs = epoch_secs;
            return;
        }
        if self.count == 0 {
            self.first_action_id = action_id.clone();
        }
        self.count += 1;
        self.last_action_id = action_id.clone();
        self.evidence_action_ids.push(action_id);
        self.last_active_secs = epoch_secs;
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

struct PluginState {
    /// 按 (trace_id, tool_name) 维护的状态
    states: BTreeMap<StateKey, ToolFailureState>,
    /// 配置
    threshold: u32,
    cooldown_secs: u64,
    state_ttl_secs: u64,
    desensitization: String,
    monitored_tools: Vec<String>,
    ignored_tools: Vec<String>,
    policy_denied_counts_as_failure: bool,
    tool_name_format: String,
    /// 当前 epoch 秒（由 batch 中的第一条记录近似）
    current_epoch_secs: u64,
}

impl PluginState {
    fn new() -> Self {
        Self {
            states: BTreeMap::new(),
            threshold: DEFAULT_THRESHOLD,
            cooldown_secs: DEFAULT_COOLDOWN_SECS,
            state_ttl_secs: DEFAULT_STATE_TTL_SECS,
            desensitization: DEFAULT_DESENSITIZATION.to_string(),
            monitored_tools: Vec::new(),
            ignored_tools: Vec::new(),
            policy_denied_counts_as_failure: DEFAULT_POLICY_DENIED_COUNTS_AS_FAILURE,
            tool_name_format: DEFAULT_TOOL_NAME_FORMAT.to_string(),
            current_epoch_secs: 0,
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

    /// 处理一个 CommandInvocation 事件
    /// `bare_tool_name` 用于过滤（monitored_tools / ignored_tools 匹配）
    /// `effective_tool_name` 用于状态键和告警输出（可能是完整命令行）
    fn process_command(
        &mut self,
        trace_id: &str,
        action_id: &str,
        bare_tool_name: &str,
        effective_tool_name: &str,
        tool_args: &str,
        status: &str,
        exit_code: &str,
        failure_summary: &str,
    ) -> Option<String> {
        if !self.should_monitor(bare_tool_name) {
            return None;
        }

        // in_progress 状态不参与计数（既不算成功也不算失败）
        if status == "in_progress" {
            return None;
        }

        let key: StateKey = (trace_id.to_string(), effective_tool_name.to_string());
        let is_success = Self::is_success_status(status, exit_code);

        if is_success {
            // 成功：重置计数器
            if let Some(state) = self.states.get_mut(&key) {
                state.record_success(self.current_epoch_secs);
            }
            return None;
        }

        // 失败：更新或创建状态
        // 策略拒绝检查：如果配置为不计入失败，则跳过
        if Self::is_policy_denied(failure_summary) && !self.policy_denied_counts_as_failure {
            return None;
        }

        let state = self
            .states
            .entry(key.clone())
            .or_insert_with(|| ToolFailureState::new(self.current_epoch_secs));

        state.record_failure(action_id.to_string(), self.current_epoch_secs);

        // 检查是否触发告警
        if state.count >= self.threshold {
            // 冷却检查
            if self.cooldown_secs > 0
                && state.last_alert_secs > 0
                && self.current_epoch_secs - state.last_alert_secs < self.cooldown_secs
            {
                return None; // 冷却中，不重复告警
            }

            state.last_alert_secs = self.current_epoch_secs;

            // 提取 state 字段，避免同时持有可变借用和不可变借用
            let count = state.count;
            let first_action_id = state.first_action_id.clone();
            let last_action_id = state.last_action_id.clone();
            let evidence_action_ids = state.evidence_action_ids.clone();
            let threshold = self.threshold;
            let desensitization = self.desensitization.clone();

            let alert = Self::build_alert_static(
                trace_id,
                effective_tool_name,
                tool_args,
                count,
                threshold,
                &desensitization,
                &first_action_id,
                &last_action_id,
                &evidence_action_ids,
                failure_summary,
                self.current_epoch_secs,
            );
            return Some(alert);
        }

        None
    }

    /// 判定成功/失败
    /// 遵循 AcTrail 的 process_exit_status() 逻辑：
    ///   exit_code=0 或 None → Success
    ///   exit_code≠0 → Error
    fn is_success_status(status: &str, exit_code: &str) -> bool {
        // status 为 "success" 即为成功
        if status == "success" {
            return true;
        }
        // exit_code 为 "0" 或空即为成功
        if exit_code.is_empty() || exit_code == "0" {
            return true;
        }
        false
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
        let now = self.current_epoch_secs;
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

        // 从 batch 中提取真实时间戳（eBPF start_time），替代递增计数器
        for action in &batch.semantic_actions {
            if let Some(ts) = action
                .attributes
                .iter()
                .find(|attr| attr.key == "action.start_time")
                .and_then(|attr| attr.value.parse::<u64>().ok())
            {
                if ts > state.current_epoch_secs {
                    state.current_epoch_secs = ts;
                }
                break;
            }
        }

        // 遍历 semantic_actions，遇一条处理一条
        let mut observed: u64 = 0;
        for action in &batch.semantic_actions {
            // 只处理 CommandInvocation（WIT 传输的 kind 字符串为 "command.invocation"）
            if action.kind != "command.invocation" {
                continue;
            }

            // 提取工具名：优先从 command.tool.name，fallback 到 process.executable / command.line
            let bare_tool_name = action
                .attributes
                .iter()
                .find(|attr| attr.key == "command.tool.name")
                .map(|attr| attr.value.as_str())
                .or_else(|| {
                    // Fallback: 从 process.executable 提取文件名（如 /usr/bin/ls → ls）
                    action
                        .attributes
                        .iter()
                        .find(|attr| attr.key == "process.executable")
                        .map(|attr| attr.value.as_str())
                        .and_then(|exec| exec.rsplit('/').next().filter(|s| !s.is_empty()))
                })
                .or_else(|| {
                    // Fallback: 从 command.line 提取第一个单词（如 "ls /tmp" → "ls"）
                    action
                        .attributes
                        .iter()
                        .find(|attr| attr.key == "command.line")
                        .map(|attr| attr.value.as_str())
                        .and_then(|line| line.split_whitespace().next().filter(|s| !s.is_empty()))
                })
                .unwrap_or("unknown");

            // 提取工具参数：从 command.line 属性获取
            let tool_args = action
                .attributes
                .iter()
                .find(|attr| attr.key == "command.line")
                .map(|attr| attr.value.as_str())
                .unwrap_or("");

            // 提取 exit_code 和 failure_summary
            let exit_code = action
                .attributes
                .iter()
                .find(|attr| attr.key == "command.exit_code")
                .map(|attr| attr.value.as_str())
                .unwrap_or("");

            let failure_summary = action
                .attributes
                .iter()
                .find(|attr| attr.key == "command.failure.summary")
                .map(|attr| attr.value.as_str())
                .unwrap_or("");

            // 根据 tool_name_format 决定用于状态键和告警的工具名
            // "bare": 仅工具名（如 "ls"），不同参数的调用共享同一计数器
            // "full": 完整命令行（如 "ls /aaa"），不同参数的独立计数
            let effective_tool_name = if state.tool_name_format == "full" && !tool_args.is_empty() {
                tool_args
            } else {
                bare_tool_name
            };

            // 处理事件
            if let Some(alert_json) = state.process_command(
                &action.trace_id,
                &action.action_id,
                bare_tool_name,
                effective_tool_name,
                tool_args,
                &action.status,
                exit_code,
                failure_summary,
            ) {
                // 通过 alert-write 接口提交告警到 daemon
                if let Ok(trace_ctx) =
                    actrail::plugin::observation_context_read::trace_context_get()
                {
                    let alert_token = trace_ctx.alert_token.unwrap_or_default();
                    let draft = actrail::plugin::types::AlertDraft {
                        definition_key: "consecutive-failure".to_string(),
                        payload_json: alert_json,
                        deduplication_key: None,
                    };
                    let request = actrail::plugin::types::AlertWriteRequest {
                        trace_id: action.trace_id.clone(),
                        alert_token,
                        draft,
                    };
                    let _ = actrail::plugin::alert_write::submit(&request);
                }
            }

            observed += 1;
        }

        // 清理过期状态
        state.cleanup_expired_states();

        Ok(ObservationReport {
            observed_records: observed,
            dropped_records: 0,
        })
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
