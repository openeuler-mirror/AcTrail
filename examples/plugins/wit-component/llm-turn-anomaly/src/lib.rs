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
    AlertDraft, AlertWriteRequest, ConfigReadStatus, LlmExchangeRecord, TraceActivityContext,
};
use exports::actrail::plugin::observation_consumer::{
    Guest as ObservationGuest, ObservationBatch, ObservationReport,
};
use exports::actrail::plugin::post_trace_analyzer::{Guest as PostTraceGuest, PostTraceTask};

const ALERT_KEY_FREQUENCY: &str = "llm-high-frequency";
const ALERT_KEY_CONSECUTIVE_RETRY: &str = "llm-consecutive-retry";
const ALERT_KEY_REPEATED_SIMILAR: &str = "llm-repeated-similar";
const ALERT_KEY_ERROR_RATIO: &str = "llm-error-ratio";
const ALERT_KEY_CONTEXT_GROWTH: &str = "llm-context-growth";
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
    plugin: Option<LlmTurnAnomalyPlugin>,
}

impl RuntimeSlot {
    fn plugin(&mut self) -> Result<&mut LlmTurnAnomalyPlugin, String> {
        if self.plugin.is_none() {
            self.plugin = Some(LlmTurnAnomalyPlugin::load()?);
        }
        self.plugin
            .as_mut()
            .ok_or_else(|| "llm-turn-anomaly runtime initialization failed".to_string())
    }
}

struct LlmTurnAnomalyPlugin {
    config: LlmTurnAnomalyConfig,
    trace_states: BTreeMap<String, TraceState>,
}

struct TraceState {
    alert_token: Vec<u8>,
}

impl LlmTurnAnomalyPlugin {
    fn load() -> Result<Self, String> {
        Ok(Self {
            config: LlmTurnAnomalyConfig::load()?,
            trace_states: BTreeMap::new(),
        })
    }

    fn observe(&mut self, batch: ObservationBatch) -> Result<(), String> {
        let has_llm_activity = batch.semantic_actions.iter().any(|action| {
            matches!(
                action.kind.as_str(),
                "llm.request" | "llm.response" | "llm.call"
            )
        });
        if !has_llm_activity && !self.trace_states.contains_key(&batch.trace_id) {
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
            return self.evaluate_live(&batch.trace_id);
        }
        if self.trace_states.len() >= self.config.trace_state_max_count {
            return Err(format!(
                "llm-turn-anomaly trace state count exceeded {}",
                self.config.trace_state_max_count
            ));
        }
        self.trace_states
            .insert(batch.trace_id.clone(), TraceState { alert_token });
        self.evaluate_live(&batch.trace_id)
    }

    fn analyze(&mut self, trace_id: &str) -> Result<(), String> {
        self.trace_states.remove(trace_id);
        Ok(())
    }

    fn evaluate_live(&mut self, trace_id: &str) -> Result<(), String> {
        let Some(state) = self.trace_states.get(trace_id) else {
            return Ok(());
        };
        let context = actrail::plugin::trace_activity_read::context_get()?;
        let exchanges = self.read_all_exchanges()?;
        if exchanges.is_empty() {
            let now = actrail::plugin::observation_context_read::current_time_ms()?;
            let reevaluate_at = now
                .checked_add(250)
                .ok_or_else(|| "llm-turn-anomaly reevaluation deadline overflow".to_string())?;
            actrail::plugin::observation_context_read::request_reevaluation_at(reevaluate_at)?;
            return Ok(());
        }

        let groups = group_exchanges(&exchanges);

        if self.config.high_frequency.enabled {
            self.check_high_frequency(trace_id, &state, &context, &groups)?;
        }
        if self.config.consecutive_retry.enabled {
            self.check_consecutive_retry(trace_id, &state, &context, &groups)?;
        }
        if self.config.repeated_similar.enabled {
            self.check_repeated_similar(trace_id, &state, &context, &groups)?;
        }
        if self.config.error_ratio.enabled {
            self.check_error_ratio(trace_id, &state, &context, &groups)?;
        }
        if self.config.context_growth.enabled {
            self.check_context_growth(trace_id, &state, &context, &groups)?;
        }
        Ok(())
    }

    fn read_all_exchanges(&self) -> Result<Vec<LlmExchangeRecord>, String> {
        let mut all = Vec::new();
        let mut offset = None;
        loop {
            let requested_offset = offset;
            let page = actrail::plugin::trace_activity_read::llm_exchanges_list(
                offset,
                self.config.page_size,
            )?;
            for exchange in page.exchanges {
                all.push(exchange);
            }
            offset = checked_next_offset(requested_offset, page.next_offset, "llm-exchanges-list")?;
            if offset.is_none() {
                return Ok(all);
            }
        }
    }

    fn check_high_frequency(
        &self,
        trace_id: &str,
        state: &TraceState,
        context: &TraceActivityContext,
        groups: &BTreeMap<ExchangeGroup, Vec<&LlmExchangeRecord>>,
    ) -> Result<(), String> {
        let rule = &self.config.high_frequency;
        let mut findings = Vec::new();
        let mut total_count = 0usize;

        for (group, exchanges) in groups {
            if exchanges.len() < rule.min_exchanges {
                continue;
            }
            let mut sorted = exchanges.clone();
            sorted.sort_by_key(|e| e.started_at);
            let mut window_start = 0usize;
            for window_end in 0..sorted.len() {
                while sorted[window_end].started_at - sorted[window_start].started_at
                    > rule.window_size_ms
                {
                    window_start += 1;
                }
                let count = window_end - window_start + 1;
                if count >= rule.threshold {
                    total_count += 1;
                    if findings.len() < self.config.finding_max_count {
                        findings.push(HighFrequencyFinding {
                            process_id: group.process_id.clone(),
                            model: group.model.clone(),
                            exchange_count: count,
                            window_start_ms: sorted[window_start].started_at,
                            window_end_ms: sorted[window_end].started_at,
                        });
                    }
                }
            }
        }

        if total_count > 0 {
            let truncated_count = total_count.saturating_sub(findings.len());
            let payload = HighFrequencyPayload {
                root_container_id: context.root_container_id.clone(),
                root_process_id: context.root_process_id.clone(),
                display_name: context.display_name.clone(),
                profile_name: context.profile_name.clone(),
                window_size_ms: rule.window_size_ms,
                threshold: rule.threshold,
                findings,
                truncated_count,
            };
            submit_alert(trace_id, &state.alert_token, ALERT_KEY_FREQUENCY, &payload)?;
        }
        Ok(())
    }

    fn check_consecutive_retry(
        &self,
        trace_id: &str,
        state: &TraceState,
        context: &TraceActivityContext,
        groups: &BTreeMap<ExchangeGroup, Vec<&LlmExchangeRecord>>,
    ) -> Result<(), String> {
        let rule = &self.config.consecutive_retry;
        let mut findings = Vec::new();
        let mut total_count = 0usize;

        for (group, exchanges) in groups {
            let mut sorted = exchanges.clone();
            sorted.sort_by_key(|e| e.started_at);
            let mut consecutive = 0usize;
            let mut first_idx = 0usize;

            for (i, exchange) in sorted.iter().enumerate() {
                let is_error = !exchange.response_complete;
                let size_ok = exchange.request_body_bytes >= rule.min_request_bytes as u64;
                if is_error && size_ok {
                    if consecutive == 0 {
                        first_idx = i;
                    }
                    consecutive += 1;
                } else {
                    if consecutive >= rule.consecutive_count {
                        total_count += 1;
                        if findings.len() < self.config.finding_max_count {
                            findings.push(ConsecutiveRetryFinding {
                                process_id: group.process_id.clone(),
                                model: group.model.clone(),
                                retry_length: consecutive,
                                first_action_id: sorted[first_idx].request_action_id.clone(),
                                last_action_id: sorted[i - 1].request_action_id.clone(),
                                first_started_at_ms: sorted[first_idx].started_at,
                                last_started_at_ms: sorted[i - 1].started_at,
                            });
                        }
                    }
                    consecutive = 0;
                }
            }
            if consecutive >= rule.consecutive_count {
                total_count += 1;
                if findings.len() < self.config.finding_max_count {
                    let last = sorted.len() - 1;
                    findings.push(ConsecutiveRetryFinding {
                        process_id: group.process_id.clone(),
                        model: group.model.clone(),
                        retry_length: consecutive,
                        first_action_id: sorted[first_idx].request_action_id.clone(),
                        last_action_id: sorted[last].request_action_id.clone(),
                        first_started_at_ms: sorted[first_idx].started_at,
                        last_started_at_ms: sorted[last].started_at,
                    });
                }
            }
        }

        if total_count > 0 {
            let truncated_count = total_count.saturating_sub(findings.len());
            let payload = ConsecutiveRetryPayload {
                root_container_id: context.root_container_id.clone(),
                root_process_id: context.root_process_id.clone(),
                display_name: context.display_name.clone(),
                profile_name: context.profile_name.clone(),
                consecutive_count: rule.consecutive_count,
                findings,
                truncated_count,
            };
            submit_alert(
                trace_id,
                &state.alert_token,
                ALERT_KEY_CONSECUTIVE_RETRY,
                &payload,
            )?;
        }
        Ok(())
    }

    fn check_repeated_similar(
        &self,
        trace_id: &str,
        state: &TraceState,
        context: &TraceActivityContext,
        groups: &BTreeMap<ExchangeGroup, Vec<&LlmExchangeRecord>>,
    ) -> Result<(), String> {
        let rule = &self.config.repeated_similar;
        let mut findings = Vec::new();
        let mut total_count = 0usize;

        for (group, exchanges) in groups {
            let mut sorted = exchanges.clone();
            sorted.sort_by_key(|e| e.started_at);
            let window = rule.similarity_window;

            if sorted.len() < window {
                continue;
            }

            let mut i = 0;
            while i + window <= sorted.len() {
                let slice = &sorted[i..i + window];
                let mut representative_idx = 0usize;
                let mut max_repeat = 1usize;

                let mut j = 0;
                while j < slice.len() {
                    let mut run_count = 1usize;
                    let mut k = j + 1;
                    while k < slice.len() {
                        if similar_requests(
                            slice[j],
                            slice[k],
                            rule.similarity_tolerance_ratio_per_mille,
                        ) {
                            run_count += 1;
                            k += 1;
                        } else {
                            break;
                        }
                    }
                    if run_count > max_repeat {
                        max_repeat = run_count;
                        representative_idx = j;
                    }
                    j = k;
                }

                if max_repeat >= rule.min_repeat_count {
                    let rep = slice[representative_idx];
                    let last_in_run = slice[representative_idx + max_repeat - 1];
                    total_count += 1;
                    if findings.len() < self.config.finding_max_count {
                        findings.push(RepeatedSimilarFinding {
                            process_id: group.process_id.clone(),
                            model: group.model.clone(),
                            repeat_count: max_repeat,
                            representative_action_id: rep.request_action_id.clone(),
                            representative_request_bytes: rep.request_body_bytes,
                            first_started_at_ms: rep.started_at,
                            last_started_at_ms: last_in_run.started_at,
                        });
                    }
                    i += max_repeat;
                } else {
                    i += 1;
                }
            }
        }

        if total_count > 0 {
            let truncated_count = total_count.saturating_sub(findings.len());
            let payload = RepeatedSimilarPayload {
                root_container_id: context.root_container_id.clone(),
                root_process_id: context.root_process_id.clone(),
                display_name: context.display_name.clone(),
                profile_name: context.profile_name.clone(),
                similarity_window: rule.similarity_window,
                similarity_tolerance_ratio_per_mille: rule.similarity_tolerance_ratio_per_mille,
                min_repeat_count: rule.min_repeat_count,
                findings,
                truncated_count,
            };
            submit_alert(
                trace_id,
                &state.alert_token,
                ALERT_KEY_REPEATED_SIMILAR,
                &payload,
            )?;
        }
        Ok(())
    }

    fn check_error_ratio(
        &self,
        trace_id: &str,
        state: &TraceState,
        context: &TraceActivityContext,
        groups: &BTreeMap<ExchangeGroup, Vec<&LlmExchangeRecord>>,
    ) -> Result<(), String> {
        let rule = &self.config.error_ratio;
        let mut findings = Vec::new();
        let mut total_count = 0usize;

        for (group, exchanges) in groups {
            let total = exchanges.len();
            if total < rule.minimum_exchanges {
                continue;
            }
            let error_count = exchanges.iter().filter(|e| !e.response_complete).count();
            if error_count == 0 {
                continue;
            }
            let actual_ratio = (error_count as u64) * 1000 / (total as u64);
            if actual_ratio >= rule.error_ratio_per_mille {
                total_count += 1;
                if findings.len() < self.config.finding_max_count {
                    findings.push(ErrorRatioFinding {
                        process_id: group.process_id.clone(),
                        model: group.model.clone(),
                        total_exchanges: total,
                        error_count,
                        actual_ratio_per_mille: actual_ratio,
                    });
                }
            }
        }

        if total_count > 0 {
            let truncated_count = total_count.saturating_sub(findings.len());
            let payload = ErrorRatioPayload {
                root_container_id: context.root_container_id.clone(),
                root_process_id: context.root_process_id.clone(),
                display_name: context.display_name.clone(),
                profile_name: context.profile_name.clone(),
                minimum_exchanges: rule.minimum_exchanges,
                error_ratio_per_mille: rule.error_ratio_per_mille,
                findings,
                truncated_count,
            };
            submit_alert(
                trace_id,
                &state.alert_token,
                ALERT_KEY_ERROR_RATIO,
                &payload,
            )?;
        }
        Ok(())
    }

    fn check_context_growth(
        &self,
        trace_id: &str,
        state: &TraceState,
        context: &TraceActivityContext,
        groups: &BTreeMap<ExchangeGroup, Vec<&LlmExchangeRecord>>,
    ) -> Result<(), String> {
        let rule = &self.config.context_growth;
        let mut findings = Vec::new();
        let mut total_count = 0usize;

        for (_group, exchanges) in groups {
            let mut sorted = exchanges.clone();
            sorted.sort_by_key(|e| e.started_at);
            let mut history: Vec<u64> = Vec::new();

            for exchange in &sorted {
                let bytes = exchange.request_body_bytes;
                let baseline = if history.len() >= rule.minimum_samples {
                    Some(median(&history))
                } else {
                    None
                };
                let triggered = if bytes >= rule.minimum_growth_bytes
                    && baseline.is_some_and(|b| {
                        b >= rule.minimum_baseline_bytes
                            && bytes.saturating_sub(b) >= rule.minimum_growth_bytes
                            && u128::from(bytes) * 1000
                                >= u128::from(b) * u128::from(rule.growth_ratio_per_mille)
                    }) {
                    Some("relative-growth")
                } else {
                    None
                };
                if let Some(reason) = triggered {
                    let b = baseline.unwrap_or(0);
                    let ratio = if b > 0 {
                        let r = u128::from(bytes) * 1000 / u128::from(b);
                        r.min(u128::from(u64::MAX)) as u64
                    } else {
                        0
                    };
                    total_count += 1;
                    if findings.len() < self.config.finding_max_count {
                        findings.push(ContextGrowthFinding {
                            action_id: exchange.request_action_id.clone(),
                            call_action_id: exchange.call_action_id.clone(),
                            process_id: exchange.process_id.clone(),
                            model: exchange.model.clone(),
                            observed_bytes: bytes,
                            baseline_median_bytes: b,
                            observed_ratio_per_mille: ratio,
                            started_at_ms: exchange.started_at,
                        });
                    }
                    let _ = reason;
                }
                history.push(bytes);
                if history.len() > rule.window_size {
                    history.remove(0);
                }
            }
        }

        if total_count > 0 {
            let truncated_count = total_count.saturating_sub(findings.len());
            let payload = ContextGrowthPayload {
                root_container_id: context.root_container_id.clone(),
                root_process_id: context.root_process_id.clone(),
                display_name: context.display_name.clone(),
                profile_name: context.profile_name.clone(),
                growth_ratio_per_mille: rule.growth_ratio_per_mille,
                minimum_baseline_bytes: rule.minimum_baseline_bytes,
                minimum_growth_bytes: rule.minimum_growth_bytes,
                window_size: rule.window_size,
                minimum_samples: rule.minimum_samples,
                findings,
                truncated_count,
            };
            submit_alert(
                trace_id,
                &state.alert_token,
                ALERT_KEY_CONTEXT_GROWTH,
                &payload,
            )?;
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LlmTurnAnomalyConfig {
    high_frequency: HighFrequencyRule,
    consecutive_retry: ConsecutiveRetryRule,
    repeated_similar: RepeatedSimilarRule,
    error_ratio: ErrorRatioRule,
    context_growth: ContextGrowthRule,
    page_size: u32,
    trace_state_max_count: usize,
    finding_max_count: usize,
}

impl LlmTurnAnomalyConfig {
    fn load() -> Result<Self, String> {
        let mut bytes = Vec::new();
        let mut offset = 0_u64;
        loop {
            let chunk = actrail::plugin::host::read_config(offset, CONFIG_CHUNK_BYTES);
            match chunk.status {
                ConfigReadStatus::Ok => {}
                ConfigReadStatus::NotConfigured => {
                    return Err("llm-turn-anomaly plugin config is required".to_string());
                }
                ConfigReadStatus::TooLarge => {
                    return Err("llm-turn-anomaly plugin config exceeds host limit".to_string());
                }
            }
            if chunk.offset != offset {
                return Err("llm-turn-anomaly config chunk offset mismatch".to_string());
            }
            bytes.extend_from_slice(&chunk.bytes);
            if bytes.len() > CONFIG_MAX_BYTES {
                return Err("llm-turn-anomaly plugin config exceeds 16384 bytes".to_string());
            }
            let Some(next_offset) = chunk.next_offset else {
                break;
            };
            if next_offset <= offset {
                return Err("llm-turn-anomaly config next offset did not advance".to_string());
            }
            offset = next_offset;
        }
        let raw = core::str::from_utf8(&bytes)
            .map_err(|error| format!("llm-turn-anomaly config is not UTF-8: {error}"))?;
        let config = serde_json::from_str::<Self>(raw)
            .map_err(|error| format!("parse llm-turn-anomaly config failed: {error}"))?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), String> {
        if self.high_frequency.window_size_ms == 0 {
            return Err("high_frequency.window_size_ms must be greater than zero".into());
        }
        if self.high_frequency.threshold < 1 {
            return Err("high_frequency.threshold must be at least 1".into());
        }
        if self.high_frequency.min_exchanges < 1 {
            return Err("high_frequency.min_exchanges must be at least 1".into());
        }
        if self.consecutive_retry.consecutive_count < 2 {
            return Err("consecutive_retry.consecutive_count must be at least 2".into());
        }
        if self.repeated_similar.similarity_window < 2 {
            return Err("repeated_similar.similarity_window must be at least 2".into());
        }
        if self.repeated_similar.min_repeat_count < 2 {
            return Err("repeated_similar.min_repeat_count must be at least 2".into());
        }
        if self.error_ratio.minimum_exchanges < 1 {
            return Err("error_ratio.minimum_exchanges must be at least 1".into());
        }
        if self.error_ratio.error_ratio_per_mille < 1
            || self.error_ratio.error_ratio_per_mille > 1000
        {
            return Err("error_ratio.error_ratio_per_mille must be between 1 and 1000".into());
        }
        if self.context_growth.growth_ratio_per_mille <= 1000 {
            return Err("context_growth.growth_ratio_per_mille must be greater than 1000".into());
        }
        if self.context_growth.window_size == 0 || self.context_growth.window_size > 64 {
            return Err("context_growth.window_size must be between 1 and 64".into());
        }
        if self.context_growth.minimum_samples == 0
            || self.context_growth.minimum_samples > self.context_growth.window_size
        {
            return Err("context_growth.minimum_samples must be between 1 and window_size".into());
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
struct HighFrequencyRule {
    enabled: bool,
    window_size_ms: u64,
    threshold: usize,
    min_exchanges: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConsecutiveRetryRule {
    enabled: bool,
    consecutive_count: usize,
    min_request_bytes: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RepeatedSimilarRule {
    enabled: bool,
    similarity_window: usize,
    min_repeat_count: usize,
    similarity_tolerance_ratio_per_mille: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ErrorRatioRule {
    enabled: bool,
    minimum_exchanges: usize,
    error_ratio_per_mille: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ContextGrowthRule {
    enabled: bool,
    growth_ratio_per_mille: u64,
    minimum_baseline_bytes: u64,
    minimum_growth_bytes: u64,
    window_size: usize,
    minimum_samples: usize,
}

#[derive(Eq, Ord, PartialEq, PartialOrd)]
struct ExchangeGroup {
    process_id: String,
    model: Option<String>,
}

fn group_exchanges<'a>(
    exchanges: &'a [LlmExchangeRecord],
) -> BTreeMap<ExchangeGroup, Vec<&'a LlmExchangeRecord>> {
    let mut groups: BTreeMap<ExchangeGroup, Vec<&LlmExchangeRecord>> = BTreeMap::new();
    for exchange in exchanges {
        let group = ExchangeGroup {
            process_id: exchange.process_id.clone(),
            model: exchange.model.clone(),
        };
        groups.entry(group).or_default().push(exchange);
    }
    groups
}

fn similar_requests(
    a: &LlmExchangeRecord,
    b: &LlmExchangeRecord,
    tolerance_per_mille: u64,
) -> bool {
    if a.process_id != b.process_id {
        return false;
    }
    if a.model != b.model {
        return false;
    }
    let a_bytes = a.request_body_bytes;
    let b_bytes = b.request_body_bytes;
    if a_bytes == b_bytes {
        return true;
    }
    if tolerance_per_mille == 0 {
        return false;
    }
    let (larger, smaller) = if a_bytes >= b_bytes {
        (a_bytes, b_bytes)
    } else {
        (b_bytes, a_bytes)
    };
    if smaller == 0 {
        return larger == 0;
    }
    let diff = larger - smaller;
    diff * 1000 <= larger * tolerance_per_mille
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
            deduplication_key: None,
        },
    })
}

#[derive(Serialize)]
struct HighFrequencyFinding {
    process_id: String,
    model: Option<String>,
    exchange_count: usize,
    window_start_ms: u64,
    window_end_ms: u64,
}

#[derive(Serialize)]
struct HighFrequencyPayload {
    root_container_id: Option<String>,
    root_process_id: String,
    display_name: String,
    profile_name: String,
    window_size_ms: u64,
    threshold: usize,
    findings: Vec<HighFrequencyFinding>,
    truncated_count: usize,
}

#[derive(Serialize)]
struct ConsecutiveRetryFinding {
    process_id: String,
    model: Option<String>,
    retry_length: usize,
    first_action_id: String,
    last_action_id: String,
    first_started_at_ms: u64,
    last_started_at_ms: u64,
}

#[derive(Serialize)]
struct ConsecutiveRetryPayload {
    root_container_id: Option<String>,
    root_process_id: String,
    display_name: String,
    profile_name: String,
    consecutive_count: usize,
    findings: Vec<ConsecutiveRetryFinding>,
    truncated_count: usize,
}

#[derive(Serialize)]
struct RepeatedSimilarFinding {
    process_id: String,
    model: Option<String>,
    repeat_count: usize,
    representative_action_id: String,
    representative_request_bytes: u64,
    first_started_at_ms: u64,
    last_started_at_ms: u64,
}

#[derive(Serialize)]
struct RepeatedSimilarPayload {
    root_container_id: Option<String>,
    root_process_id: String,
    display_name: String,
    profile_name: String,
    similarity_window: usize,
    similarity_tolerance_ratio_per_mille: u64,
    min_repeat_count: usize,
    findings: Vec<RepeatedSimilarFinding>,
    truncated_count: usize,
}

#[derive(Serialize)]
struct ErrorRatioFinding {
    process_id: String,
    model: Option<String>,
    total_exchanges: usize,
    error_count: usize,
    actual_ratio_per_mille: u64,
}

#[derive(Serialize)]
struct ErrorRatioPayload {
    root_container_id: Option<String>,
    root_process_id: String,
    display_name: String,
    profile_name: String,
    minimum_exchanges: usize,
    error_ratio_per_mille: u64,
    findings: Vec<ErrorRatioFinding>,
    truncated_count: usize,
}

#[derive(Serialize)]
struct ContextGrowthFinding {
    action_id: String,
    call_action_id: String,
    process_id: String,
    model: Option<String>,
    observed_bytes: u64,
    baseline_median_bytes: u64,
    observed_ratio_per_mille: u64,
    started_at_ms: u64,
}

#[derive(Serialize)]
struct ContextGrowthPayload {
    root_container_id: Option<String>,
    root_process_id: String,
    display_name: String,
    profile_name: String,
    growth_ratio_per_mille: u64,
    minimum_baseline_bytes: u64,
    minimum_growth_bytes: u64,
    window_size: usize,
    minimum_samples: usize,
    findings: Vec<ContextGrowthFinding>,
    truncated_count: usize,
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
