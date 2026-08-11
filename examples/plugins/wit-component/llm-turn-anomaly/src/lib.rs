#![no_std]

extern crate alloc;

use alloc::alloc::{Layout, alloc, realloc};
use alloc::collections::{BTreeMap, VecDeque};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use serde::{Deserialize, Serialize};
use spin::Mutex;

mod detectors;

use detectors::{DetectorState, ResponseOutcome};

wit_bindgen::generate!({
    path: "../../../../crates/core/plugin_system/wit",
    world: "activity-anomaly-plugin",
});

use actrail::plugin::types::{AlertDraft, AlertWriteRequest, ConfigReadStatus};
use exports::actrail::plugin::observation_consumer::{
    Guest as ObservationGuest, ObservationBatch, ObservationReport,
};
use exports::actrail::plugin::post_trace_analyzer::{Guest as PostTraceGuest, PostTraceTask};

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
    next_exchange_offset: u64,
    last_request_action_id: Option<String>,
    pending_responses: VecDeque<PendingExchange>,
    detectors: DetectorState,
}

struct PendingExchange {
    offset: u64,
    request_action_id: String,
    outcome: Option<ResponseOutcome>,
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
        self.trace_states.insert(
            batch.trace_id.clone(),
            TraceState {
                alert_token,
                next_exchange_offset: 0,
                last_request_action_id: None,
                pending_responses: VecDeque::new(),
                detectors: DetectorState::default(),
            },
        );
        self.evaluate_live(&batch.trace_id)
    }

    fn analyze(&mut self, trace_id: &str) -> Result<(), String> {
        self.trace_states.remove(trace_id);
        Ok(())
    }

    fn evaluate_live(&mut self, trace_id: &str) -> Result<(), String> {
        let Some(state) = self.trace_states.get_mut(trace_id) else {
            return Ok(());
        };
        let alert_token = state.alert_token.clone();
        let context = actrail::plugin::trace_activity_read::context_get()?;
        read_new_exchanges(&self.config, state)?;
        if state.next_exchange_offset == 0 {
            let now = actrail::plugin::observation_context_read::current_time_ms()?;
            let reevaluate_at = now
                .checked_add(250)
                .ok_or_else(|| "llm-turn-anomaly reevaluation deadline overflow".to_string())?;
            actrail::plugin::observation_context_read::request_reevaluation_at(reevaluate_at)?;
            return Ok(());
        }

        state
            .detectors
            .evaluate(trace_id, &alert_token, &context, &self.config)
    }
}

fn read_new_exchanges(config: &LlmTurnAnomalyConfig, state: &mut TraceState) -> Result<(), String> {
    let mut offset = if state.next_exchange_offset == 0 {
        None
    } else {
        Some(state.next_exchange_offset - 1)
    };
    let mut verify_checkpoint = state.next_exchange_offset > 0;

    loop {
        let requested_offset = offset;
        let page = actrail::plugin::trace_activity_read::llm_exchanges_list(
            requested_offset,
            config.page_size,
        )?;
        let mut exchanges = page.exchanges.into_iter();

        if verify_checkpoint {
            let checkpoint = exchanges.next().ok_or_else(|| {
                "llm-turn-anomaly incremental exchange checkpoint disappeared".to_string()
            })?;
            if state.last_request_action_id.as_ref() != Some(&checkpoint.request_action_id) {
                return Err("llm-turn-anomaly incremental exchange checkpoint changed".to_string());
            }
            verify_checkpoint = false;
        }

        for exchange in exchanges {
            state.pending_responses.push_back(PendingExchange {
                offset: state.next_exchange_offset,
                request_action_id: exchange.request_action_id.clone(),
                outcome: ResponseOutcome::from_exchange(&exchange),
            });
            state.detectors.observe(config, &exchange)?;
            state.next_exchange_offset = state
                .next_exchange_offset
                .checked_add(1)
                .ok_or_else(|| "llm-turn-anomaly exchange offset overflow".to_string())?;
            state.last_request_action_id = Some(exchange.request_action_id.clone());
        }

        let Some(next_offset) = page.next_offset else {
            break;
        };
        if requested_offset.is_some_and(|current| next_offset <= current) {
            return Err("llm-exchanges-list next offset did not advance".to_string());
        }
        if next_offset != state.next_exchange_offset {
            return Err("llm-turn-anomaly incremental exchange offset mismatch".to_string());
        }
        offset = Some(next_offset);
    }

    refresh_pending_responses(state)?;
    while state
        .pending_responses
        .front()
        .is_some_and(|pending| pending.outcome.is_some())
    {
        let pending = state
            .pending_responses
            .pop_front()
            .ok_or_else(|| "llm-turn-anomaly pending response disappeared".to_string())?;
        if let Some(outcome) = pending.outcome {
            state.detectors.observe_response(config, &outcome)?;
        }
    }
    Ok(())
}

fn refresh_pending_responses(state: &mut TraceState) -> Result<(), String> {
    for index in 0..state.pending_responses.len() {
        let Some(pending) = state.pending_responses.get(index) else {
            return Err("llm-turn-anomaly pending response index disappeared".to_string());
        };
        if pending.outcome.is_some() {
            continue;
        }
        let offset = pending.offset;
        let request_action_id = pending.request_action_id.clone();
        let page = actrail::plugin::trace_activity_read::llm_exchanges_list(Some(offset), 1)?;
        let exchange = page.exchanges.into_iter().next().ok_or_else(|| {
            "llm-turn-anomaly pending exchange disappeared while awaiting response".to_string()
        })?;
        if exchange.request_action_id != request_action_id {
            return Err("llm-turn-anomaly pending exchange changed while awaiting response".into());
        }
        if let Some(outcome) = ResponseOutcome::from_exchange(&exchange) {
            let pending = state.pending_responses.get_mut(index).ok_or_else(|| {
                "llm-turn-anomaly pending response index disappeared".to_string()
            })?;
            pending.outcome = Some(outcome);
        }
    }
    Ok(())
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
        if self.repeated_similar.similarity_tolerance_ratio_per_mille > 500 {
            return Err(
                "repeated_similar.similarity_tolerance_ratio_per_mille must be at most 500".into(),
            );
        }
        if self.error_ratio.minimum_exchanges < 1 {
            return Err("error_ratio.minimum_exchanges must be at least 1".into());
        }
        if self.error_ratio.error_ratio_per_mille < 1
            || self.error_ratio.error_ratio_per_mille > 1000
        {
            return Err("error_ratio.error_ratio_per_mille must be between 1 and 1000".into());
        }
        if self.error_ratio.window_size < 1 || self.error_ratio.window_size > 1000 {
            return Err("error_ratio.window_size must be between 1 and 1000".into());
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
    window_size: usize,
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

fn submit_alert_with_dedup(
    trace_id: &str,
    alert_token: &[u8],
    definition_key: &str,
    payload: &impl Serialize,
    deduplication_key: &str,
) -> Result<(), String> {
    let payload_json = serde_json::to_string(payload)
        .map_err(|error| format!("serialize {definition_key} alert payload failed: {error}"))?;
    actrail::plugin::alert_write::submit(&AlertWriteRequest {
        trace_id: trace_id.to_string(),
        alert_token: alert_token.to_vec(),
        draft: AlertDraft {
            definition_key: definition_key.to_string(),
            payload_json,
            deduplication_key: Some(deduplication_key.to_string()),
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
