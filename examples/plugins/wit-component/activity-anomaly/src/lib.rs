#![no_std]

extern crate alloc;

use alloc::alloc::{Layout, alloc, realloc};
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use serde::{Deserialize, Serialize};
use spin::Mutex;

wit_bindgen::generate!({
    path: "../../../../crates/core/plugin_system/wit",
    world: "activity-anomaly-plugin",
});

use actrail::plugin::types::{
    AlertDraft, AlertWriteRequest, CommandExecutionRecord, ConfigReadStatus, LlmExchangeRecord,
    ObservationEventFamily, TraceActivityContext,
};
use exports::actrail::plugin::observation_consumer::{
    Guest as ObservationGuest, ObservationBatch, ObservationReport,
};
use exports::actrail::plugin::post_trace_analyzer::{Guest as PostTraceGuest, PostTraceTask};

const REQUEST_ALERT_KEY: &str = "llm-request-growth";
const RESPONSE_ALERT_KEY: &str = "llm-response-growth";
const COMMAND_ALERT_KEY: &str = "command-duration-exceeded";
const CONFIG_CHUNK_BYTES: u64 = 4096;
const CONFIG_MAX_BYTES: usize = 16384;

#[global_allocator]
static ALLOCATOR: wee_alloc::WeeAlloc = wee_alloc::WeeAlloc::INIT;

struct Component;

static RUNTIME: Mutex<RuntimeSlot> = Mutex::new(RuntimeSlot { plugin: None });

impl ObservationGuest for Component {
    fn consume(batch: ObservationBatch) -> Result<ObservationReport, String> {
        wit_bindgen::rt::maybe_link_cabi_realloc();
        let observed_records = batch.semantic_actions.len() as u64;
        RUNTIME.lock().plugin()?.observe(batch)?;
        Ok(ObservationReport {
            observed_records,
            dropped_records: 0,
        })
    }
}

impl PostTraceGuest for Component {
    fn analyze(task: PostTraceTask) -> Result<(), String> {
        wit_bindgen::rt::maybe_link_cabi_realloc();
        RUNTIME.lock().plugin()?.analyze(&task.trace_id)
    }
}

struct RuntimeSlot {
    plugin: Option<ActivityAnomalyPlugin>,
}

impl RuntimeSlot {
    fn plugin(&mut self) -> Result<&mut ActivityAnomalyPlugin, String> {
        if self.plugin.is_none() {
            self.plugin = Some(ActivityAnomalyPlugin::load()?);
        }
        self.plugin
            .as_mut()
            .ok_or_else(|| "activity-anomaly runtime initialization failed".to_string())
    }
}

struct ActivityAnomalyPlugin {
    config: ActivityAnomalyConfig,
    trace_states: BTreeMap<String, TraceState>,
}

struct TraceState {
    alert_token: Vec<u8>,
    reported: ReportedAlerts,
}

#[derive(Clone, Copy, Default)]
struct ReportedAlerts {
    request_growth: bool,
    response_growth: bool,
    command_duration: bool,
}

impl ReportedAlerts {
    fn all_enabled(self, config: &ActivityAnomalyConfig) -> bool {
        (!config.request_growth.enabled || self.request_growth)
            && (!config.response_growth.enabled || self.response_growth)
            && (!config.command_duration.enabled || self.command_duration)
    }
}

impl ActivityAnomalyPlugin {
    fn load() -> Result<Self, String> {
        Ok(Self {
            config: ActivityAnomalyConfig::load()?,
            trace_states: BTreeMap::new(),
        })
    }

    fn observe(&mut self, batch: ObservationBatch) -> Result<(), String> {
        let has_relevant_activity = batch.semantic_actions.iter().any(|action| {
            matches!(
                action.kind.as_str(),
                "llm.call" | "llm.request" | "llm.response" | "command.invocation"
            )
        });
        let can_complete_existing_activity = self.trace_states.contains_key(&batch.trace_id)
            && batch
                .families
                .contains(&ObservationEventFamily::SemanticActionLink);
        if !has_relevant_activity && !can_complete_existing_activity {
            return Ok(());
        }
        let context = actrail::plugin::observation_context_read::trace_context_get()?;
        let alert_token = context
            .alert_token
            .ok_or_else(|| "alert token was not granted for this trace".to_string())?;
        if let Some(state) = self.trace_states.get(&batch.trace_id) {
            if state.alert_token != alert_token {
                return Err(format!(
                    "alert token changed while observing trace {}",
                    batch.trace_id
                ));
            }
        } else {
            if self.trace_states.len() >= self.config.trace_state_max_count {
                return Err(format!(
                    "activity anomaly trace state count exceeded {}",
                    self.config.trace_state_max_count
                ));
            }
            self.trace_states.insert(
                batch.trace_id.clone(),
                TraceState {
                    alert_token,
                    reported: ReportedAlerts::default(),
                },
            );
        }
        self.evaluate(&batch.trace_id)
    }

    fn analyze(&mut self, trace_id: &str) -> Result<(), String> {
        if !self.trace_states.contains_key(trace_id) {
            return Ok(());
        }
        let result = self.evaluate(trace_id);
        self.trace_states.remove(trace_id);
        result
    }

    fn evaluate(&mut self, trace_id: &str) -> Result<(), String> {
        let state = self
            .trace_states
            .get(trace_id)
            .ok_or_else(|| format!("activity anomaly trace state {trace_id} is unavailable"))?;
        let alert_token = state.alert_token.clone();
        let reported = state.reported;
        if reported.all_enabled(&self.config) {
            return Ok(());
        }
        let context = actrail::plugin::trace_activity_read::context_get()?;
        let (request_payload, response_payload) =
            if reported.request_growth && reported.response_growth {
                (None, None)
            } else {
                let mut request = GrowthDetector::new(
                    "request",
                    &self.config.request_growth,
                    self.config.finding_max_count,
                );
                let mut response = GrowthDetector::new(
                    "response",
                    &self.config.response_growth,
                    self.config.finding_max_count,
                );
                self.read_llm_exchanges(&mut request, &mut response)?;
                (
                    (!reported.request_growth && request.has_findings())
                        .then(|| request.payload(&context)),
                    (!reported.response_growth && response.has_findings())
                        .then(|| response.payload(&context)),
                )
            };
        let command_payload = if reported.command_duration {
            None
        } else {
            let command_findings = self.read_commands()?;
            (command_findings.total_count > 0).then(|| {
                let truncated_count = command_findings.truncated_count();
                CommandDurationPayload {
                    root_container_id: context.root_container_id.clone(),
                    root_process_id: context.root_process_id.clone(),
                    display_name: context.display_name.clone(),
                    profile_name: context.profile_name.clone(),
                    maximum_duration_ms: self.config.command_duration.maximum_duration_ms,
                    findings: command_findings.findings,
                    truncated_count,
                }
            })
        };

        if let Some(payload) = request_payload {
            submit_alert(trace_id, &alert_token, REQUEST_ALERT_KEY, &payload)?;
            self.trace_states
                .get_mut(trace_id)
                .ok_or_else(|| {
                    format!("activity anomaly trace state {trace_id} disappeared while evaluating")
                })?
                .reported
                .request_growth = true;
        }
        if let Some(payload) = response_payload {
            submit_alert(trace_id, &alert_token, RESPONSE_ALERT_KEY, &payload)?;
            self.trace_states
                .get_mut(trace_id)
                .ok_or_else(|| {
                    format!("activity anomaly trace state {trace_id} disappeared while evaluating")
                })?
                .reported
                .response_growth = true;
        }
        if let Some(payload) = command_payload {
            submit_alert(trace_id, &alert_token, COMMAND_ALERT_KEY, &payload)?;
            self.trace_states
                .get_mut(trace_id)
                .ok_or_else(|| {
                    format!("activity anomaly trace state {trace_id} disappeared while evaluating")
                })?
                .reported
                .command_duration = true;
        }
        Ok(())
    }

    fn read_llm_exchanges(
        &self,
        request: &mut GrowthDetector<'_>,
        response: &mut GrowthDetector<'_>,
    ) -> Result<(), String> {
        if !request.rule.enabled && !response.rule.enabled {
            return Ok(());
        }
        let mut offset = None;
        loop {
            let requested_offset = offset;
            let page = actrail::plugin::trace_activity_read::llm_exchanges_list(
                offset,
                self.config.page_size,
            )?;
            for exchange in page.exchanges {
                request.observe(&exchange, GrowthDirection::Request);
                response.observe(&exchange, GrowthDirection::Response);
            }
            offset = checked_next_offset(requested_offset, page.next_offset, "llm-exchanges-list")?;
            if offset.is_none() {
                return Ok(());
            }
        }
    }

    fn read_commands(&self) -> Result<CommandFindingSet, String> {
        let mut findings = CommandFindingSet::default();
        if !self.config.command_duration.enabled {
            return Ok(findings);
        }
        let mut offset = None;
        loop {
            let requested_offset = offset;
            let page = actrail::plugin::trace_activity_read::command_executions_list(
                offset,
                self.config.page_size,
            )?;
            for command in page.commands {
                self.observe_command(command, &mut findings)?;
            }
            offset = checked_next_offset(
                requested_offset,
                page.next_offset,
                "command-executions-list",
            )?;
            if offset.is_none() {
                return Ok(findings);
            }
        }
    }

    fn observe_command(
        &self,
        command: CommandExecutionRecord,
        findings: &mut CommandFindingSet,
    ) -> Result<(), String> {
        if !command.top_level_agent_child {
            return Ok(());
        }
        let Some(ended_at_ms) = command.ended_at else {
            return Ok(());
        };
        let duration_ms = ended_at_ms
            .checked_sub(command.started_at)
            .ok_or_else(|| format!("command {} ended before it started", command.action_id))?;
        if duration_ms <= self.config.command_duration.maximum_duration_ms {
            return Ok(());
        }
        findings.total_count = findings
            .total_count
            .checked_add(1)
            .ok_or_else(|| "command finding count overflow".to_string())?;
        if findings.findings.len() < self.config.finding_max_count {
            findings.findings.push(CommandDurationFinding {
                action_id: command.action_id,
                process_id: command.process_id,
                executable: command.executable,
                command_line: command.command_line,
                started_at_ms: command.started_at,
                ended_at_ms,
                duration_ms,
                status: command.status,
                exit_code: command.exit_code,
                agent_action_id: command.agent_action_id,
            });
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ActivityAnomalyConfig {
    request_growth: GrowthRule,
    response_growth: GrowthRule,
    command_duration: CommandDurationRule,
    page_size: u32,
    trace_state_max_count: usize,
    finding_max_count: usize,
}

impl ActivityAnomalyConfig {
    fn load() -> Result<Self, String> {
        let mut bytes = Vec::new();
        let mut offset = 0_u64;
        loop {
            let chunk = actrail::plugin::host::read_config(offset, CONFIG_CHUNK_BYTES);
            match chunk.status {
                ConfigReadStatus::Ok => {}
                ConfigReadStatus::NotConfigured => {
                    return Err("activity-anomaly plugin config is required".to_string());
                }
                ConfigReadStatus::TooLarge => {
                    return Err("activity-anomaly plugin config exceeds host limit".to_string());
                }
            }
            if chunk.offset != offset {
                return Err("activity-anomaly config chunk offset mismatch".to_string());
            }
            bytes.extend_from_slice(&chunk.bytes);
            if bytes.len() > CONFIG_MAX_BYTES {
                return Err("activity-anomaly plugin config exceeds 16384 bytes".to_string());
            }
            let Some(next_offset) = chunk.next_offset else {
                break;
            };
            if next_offset <= offset {
                return Err("activity-anomaly config next offset did not advance".to_string());
            }
            offset = next_offset;
        }
        let raw = core::str::from_utf8(&bytes)
            .map_err(|error| format!("activity-anomaly config is not UTF-8: {error}"))?;
        let config = serde_json::from_str::<Self>(raw)
            .map_err(|error| format!("parse activity-anomaly config failed: {error}"))?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), String> {
        self.request_growth.validate("request_growth")?;
        self.response_growth.validate("response_growth")?;
        if self.command_duration.maximum_duration_ms == 0 {
            return Err("command_duration.maximum_duration_ms must be greater than zero".into());
        }
        if self.page_size == 0 || self.page_size > 256 {
            return Err("page_size must be between 1 and 256".into());
        }
        if self.trace_state_max_count == 0 || self.trace_state_max_count > 4096 {
            return Err("trace_state_max_count must be between 1 and 4096".into());
        }
        if self.finding_max_count == 0 || self.finding_max_count > 4096 {
            return Err("finding_max_count must be between 1 and 4096".into());
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GrowthRule {
    enabled: bool,
    window_size: usize,
    minimum_samples: usize,
    ratio_per_mille: u64,
    minimum_growth_bytes: u64,
    minimum_current_bytes: u64,
    hard_limit_bytes: u64,
}

impl GrowthRule {
    fn validate(&self, name: &str) -> Result<(), String> {
        if self.window_size == 0 || self.window_size > 64 {
            return Err(format!("{name}.window_size must be between 1 and 64"));
        }
        if self.minimum_samples == 0 || self.minimum_samples > self.window_size {
            return Err(format!(
                "{name}.minimum_samples must be between 1 and window_size"
            ));
        }
        if self.ratio_per_mille <= 1000 {
            return Err(format!("{name}.ratio_per_mille must be greater than 1000"));
        }
        if self.minimum_growth_bytes == 0
            || self.minimum_current_bytes == 0
            || self.hard_limit_bytes == 0
        {
            return Err(format!("{name} byte thresholds must be greater than zero"));
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CommandDurationRule {
    enabled: bool,
    maximum_duration_ms: u64,
}

enum GrowthDirection {
    Request,
    Response,
}

#[derive(Eq, Ord, PartialEq, PartialOrd)]
struct GrowthGroup {
    process_id: String,
    model: Option<String>,
    server_address: Option<String>,
    url_path: Option<String>,
}

struct GrowthDetector<'a> {
    direction: &'static str,
    rule: &'a GrowthRule,
    finding_max_count: usize,
    histories: BTreeMap<GrowthGroup, Vec<u64>>,
    findings: Vec<LlmGrowthFinding>,
    total_count: usize,
}

impl<'a> GrowthDetector<'a> {
    fn new(direction: &'static str, rule: &'a GrowthRule, finding_max_count: usize) -> Self {
        Self {
            direction,
            rule,
            finding_max_count,
            histories: BTreeMap::new(),
            findings: Vec::new(),
            total_count: 0,
        }
    }

    fn observe(&mut self, exchange: &LlmExchangeRecord, direction: GrowthDirection) {
        if !self.rule.enabled {
            return;
        }
        let (complete, observed_bytes, action_id) = match direction {
            GrowthDirection::Request => (
                exchange.request_complete,
                Some(exchange.request_body_bytes),
                Some(exchange.request_action_id.as_str()),
            ),
            GrowthDirection::Response => (
                exchange.response_complete,
                exchange.response_body_bytes,
                exchange.response_action_id.as_deref(),
            ),
        };
        let (Some(observed_bytes), Some(action_id)) = (observed_bytes, action_id) else {
            return;
        };
        if !complete {
            return;
        }
        let group = GrowthGroup {
            process_id: exchange.process_id.clone(),
            model: exchange.model.clone(),
            server_address: exchange.server_address.clone(),
            url_path: exchange.url_path.clone(),
        };
        let history = self.histories.entry(group).or_default();
        let baseline = (history.len() >= self.rule.minimum_samples).then(|| median(history));
        let reason = if observed_bytes >= self.rule.hard_limit_bytes {
            Some("hard-limit")
        } else if baseline.is_some_and(|baseline| {
            observed_bytes >= self.rule.minimum_current_bytes
                && observed_bytes.saturating_sub(baseline) >= self.rule.minimum_growth_bytes
                && u128::from(observed_bytes) * 1000
                    >= u128::from(baseline) * u128::from(self.rule.ratio_per_mille)
        }) {
            Some("relative-growth")
        } else {
            None
        };
        if let Some(reason) = reason {
            self.total_count = self.total_count.saturating_add(1);
            if self.findings.len() < self.finding_max_count {
                self.findings.push(LlmGrowthFinding {
                    action_id: action_id.to_string(),
                    call_action_id: exchange.call_action_id.clone(),
                    process_id: exchange.process_id.clone(),
                    model: exchange.model.clone(),
                    server_address: exchange.server_address.clone(),
                    url_path: exchange.url_path.clone(),
                    observed_bytes,
                    baseline_median_bytes: baseline,
                    observed_ratio_per_mille: baseline.filter(|baseline| *baseline > 0).map(
                        |baseline| {
                            let ratio = u128::from(observed_bytes) * 1000 / u128::from(baseline);
                            ratio.min(u128::from(u64::MAX)) as u64
                        },
                    ),
                    reason,
                    started_at_ms: exchange.started_at,
                });
            }
        }
        history.push(observed_bytes);
        if history.len() > self.rule.window_size {
            history.remove(0);
        }
    }

    fn has_findings(&self) -> bool {
        self.total_count > 0
    }

    fn payload(&self, context: &TraceActivityContext) -> LlmGrowthPayload {
        LlmGrowthPayload {
            direction: self.direction,
            root_container_id: context.root_container_id.clone(),
            root_process_id: context.root_process_id.clone(),
            display_name: context.display_name.clone(),
            profile_name: context.profile_name.clone(),
            window_size: self.rule.window_size,
            minimum_samples: self.rule.minimum_samples,
            ratio_per_mille: self.rule.ratio_per_mille,
            minimum_growth_bytes: self.rule.minimum_growth_bytes,
            minimum_current_bytes: self.rule.minimum_current_bytes,
            hard_limit_bytes: self.rule.hard_limit_bytes,
            findings: self.findings.clone(),
            truncated_count: self.total_count.saturating_sub(self.findings.len()),
        }
    }
}

fn median(values: &[u64]) -> u64 {
    let mut ordered = values.to_vec();
    ordered.sort_unstable();
    let midpoint = ordered.len() / 2;
    if ordered.len() % 2 == 1 {
        ordered[midpoint]
    } else {
        ((u128::from(ordered[midpoint - 1]) + u128::from(ordered[midpoint])) / 2) as u64
    }
}

#[derive(Clone, Serialize)]
struct LlmGrowthFinding {
    action_id: String,
    call_action_id: String,
    process_id: String,
    model: Option<String>,
    server_address: Option<String>,
    url_path: Option<String>,
    observed_bytes: u64,
    baseline_median_bytes: Option<u64>,
    observed_ratio_per_mille: Option<u64>,
    reason: &'static str,
    started_at_ms: u64,
}

#[derive(Serialize)]
struct LlmGrowthPayload {
    direction: &'static str,
    root_container_id: Option<String>,
    root_process_id: String,
    display_name: String,
    profile_name: String,
    window_size: usize,
    minimum_samples: usize,
    ratio_per_mille: u64,
    minimum_growth_bytes: u64,
    minimum_current_bytes: u64,
    hard_limit_bytes: u64,
    findings: Vec<LlmGrowthFinding>,
    truncated_count: usize,
}

#[derive(Default)]
struct CommandFindingSet {
    findings: Vec<CommandDurationFinding>,
    total_count: usize,
}

impl CommandFindingSet {
    fn truncated_count(&self) -> usize {
        self.total_count.saturating_sub(self.findings.len())
    }
}

#[derive(Serialize)]
struct CommandDurationFinding {
    action_id: String,
    process_id: String,
    executable: Option<String>,
    command_line: Option<String>,
    started_at_ms: u64,
    ended_at_ms: u64,
    duration_ms: u64,
    status: String,
    exit_code: Option<i32>,
    agent_action_id: Option<String>,
}

#[derive(Serialize)]
struct CommandDurationPayload {
    root_container_id: Option<String>,
    root_process_id: String,
    display_name: String,
    profile_name: String,
    maximum_duration_ms: u64,
    findings: Vec<CommandDurationFinding>,
    truncated_count: usize,
}

fn checked_next_offset(
    current: Option<u64>,
    next: Option<u64>,
    operation: &str,
) -> Result<Option<u64>, String> {
    if let Some(next) = next
        && current.is_some_and(|current| next <= current)
    {
        return Err(format!("{operation} next offset did not advance"));
    }
    Ok(next)
}

fn submit_alert(
    trace_id: &str,
    alert_token: &[u8],
    definition_key: &str,
    payload: &impl Serialize,
) -> Result<(), String> {
    let payload_json = serde_json::to_string(payload)
        .map_err(|error| format!("serialize {definition_key} alert payload failed: {error}"))?;
    actrail::plugin::alert_write::submit(&AlertWriteRequest {
        trace_id: trace_id.to_string(),
        alert_token: alert_token.to_vec(),
        draft: AlertDraft {
            definition_key: definition_key.to_string(),
            payload_json,
        },
    })
}

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

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {}
}

export!(Component);
