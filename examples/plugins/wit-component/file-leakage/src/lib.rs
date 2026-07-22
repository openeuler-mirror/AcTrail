#![no_std]

extern crate alloc;

use alloc::alloc::{Layout, alloc, realloc};
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use serde::Deserialize;
use spin::Mutex;

wit_bindgen::generate!({
    path: "../../../../crates/core/plugin_system/wit",
    world: "post-trace-observation-plugin",
});

use actrail::plugin::types::{
    AlertDraft, AlertWriteRequest, ConfigReadStatus, PathObservationState, TraceFileStateStatus,
};
use exports::actrail::plugin::observation_consumer::{
    Guest as ObservationGuest, ObservationBatch, ObservationReport,
};
use exports::actrail::plugin::post_trace_analyzer::{Guest as PostTraceGuest, PostTraceTask};

const ALERT_DEFINITION_KEY: &str = "file-leakage";

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
    plugin: Option<FileLeakagePlugin>,
}

impl RuntimeSlot {
    fn plugin(&mut self) -> Result<&mut FileLeakagePlugin, String> {
        if self.plugin.is_none() {
            self.plugin = Some(FileLeakagePlugin::load()?);
        }
        self.plugin
            .as_mut()
            .ok_or_else(|| "file-leakage runtime initialization failed".to_string())
    }
}

struct FileLeakagePlugin {
    config: FileLeakageConfig,
    trace_states: BTreeMap<String, TraceState>,
}

struct TraceState {
    alert_token: Vec<u8>,
    candidates: BTreeMap<String, String>,
}

impl FileLeakagePlugin {
    fn load() -> Result<Self, String> {
        Ok(Self {
            config: FileLeakageConfig::load()?,
            trace_states: BTreeMap::new(),
        })
    }

    fn observe(&mut self, batch: ObservationBatch) -> Result<(), String> {
        let has_candidate_write = batch.semantic_actions.iter().any(|action| {
            action.file_change.as_ref().is_some_and(|change| {
                change.successful && matches!(change.operation.as_str(), "write" | "writev")
            })
        });
        if !has_candidate_write {
            return Ok(());
        }
        let context = actrail::plugin::observation_context_read::trace_context_get()?;
        let working_directory = context.working_directory;
        let alert_token = context
            .alert_token
            .ok_or_else(|| "alert token was not granted for this trace".to_string())?;
        let allowed_roots = self.config.allowed_roots(working_directory.as_deref())?;
        for action in batch.semantic_actions {
            let Some(change) = action.file_change else {
                continue;
            };
            if !change.successful || !matches!(change.operation.as_str(), "write" | "writev") {
                continue;
            }
            if change.path_state != PathObservationState::Complete {
                return Err(format!(
                    "successful file write {} has no complete path",
                    action.action_id
                ));
            }
            let path = change.path.ok_or_else(|| {
                format!(
                    "successful file write {} is missing its path",
                    action.action_id
                )
            })?;
            let path = normalize_observed_path(&path, working_directory.as_deref())?;
            if path_is_allowed(&path, &allowed_roots) {
                continue;
            }
            self.save_candidate(&batch.trace_id, &alert_token, path, action.action_id)?;
        }
        Ok(())
    }

    fn save_candidate(
        &mut self,
        trace_id: &str,
        alert_token: &[u8],
        path: String,
        action_id: String,
    ) -> Result<(), String> {
        if !self.trace_states.contains_key(trace_id)
            && self.trace_states.len() >= self.config.trace_state_max_count
        {
            return Err(format!(
                "file leakage trace state count exceeded {}",
                self.config.trace_state_max_count
            ));
        }
        let state = self
            .trace_states
            .entry(trace_id.to_string())
            .or_insert_with(|| TraceState {
                alert_token: alert_token.to_vec(),
                candidates: BTreeMap::new(),
            });
        if state.alert_token != alert_token {
            return Err(format!(
                "alert token changed while observing trace {trace_id}"
            ));
        }
        if !state.candidates.contains_key(&path)
            && state.candidates.len() >= self.config.candidate_max_count
        {
            return Err(format!(
                "file leakage candidate count for trace {trace_id} exceeded {}",
                self.config.candidate_max_count
            ));
        }
        state.candidates.entry(path).or_insert(action_id);
        Ok(())
    }

    fn analyze(&mut self, trace_id: &str) -> Result<(), String> {
        let Some(state) = self.trace_states.remove(trace_id) else {
            return Ok(());
        };
        let mut residual_files = Vec::new();
        for (path, action_id) in state.candidates {
            let state = actrail::plugin::trace_file_state_read::get(&action_id)?;
            match state.status {
                TraceFileStateStatus::Exists => residual_files.push(path),
                TraceFileStateStatus::NotFound => {}
                TraceFileStateStatus::Inaccessible => {
                    return Err(format!(
                        "residual check for trace {trace_id} was inaccessible"
                    ));
                }
                TraceFileStateStatus::Unavailable => {
                    return Err(format!(
                        "residual check for trace {trace_id} was unavailable"
                    ));
                }
            }
        }
        if residual_files.is_empty() {
            return Ok(());
        }
        let payload_json = serde_json::to_string(&FileLeakagePayload { residual_files })
            .map_err(|error| format!("serialize alert payload failed: {error}"))?;
        actrail::plugin::alert_write::submit(&AlertWriteRequest {
            trace_id: trace_id.to_string(),
            alert_token: state.alert_token,
            draft: AlertDraft {
                definition_key: ALERT_DEFINITION_KEY.to_string(),
                payload_json,
            },
        })?;
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileLeakageConfig {
    include_trace_working_directory: bool,
    #[serde(default)]
    additional_allowed_roots: Vec<String>,
    trace_state_max_count: usize,
    candidate_max_count: usize,
}

impl FileLeakageConfig {
    fn load() -> Result<Self, String> {
        let mut bytes = Vec::new();
        let mut offset = 0_u64;
        loop {
            let chunk = actrail::plugin::host::read_config(offset, 4096);
            match chunk.status {
                ConfigReadStatus::Ok => {}
                ConfigReadStatus::NotConfigured => {
                    return Err("file-leakage plugin config is required".to_string());
                }
                ConfigReadStatus::TooLarge => {
                    return Err("file-leakage plugin config exceeds host limit".to_string());
                }
            }
            if chunk.offset != offset {
                return Err("file-leakage config chunk offset mismatch".to_string());
            }
            bytes.extend_from_slice(&chunk.bytes);
            let Some(next_offset) = chunk.next_offset else {
                break;
            };
            offset = next_offset;
        }
        let raw = core::str::from_utf8(&bytes)
            .map_err(|error| format!("file-leakage config is not UTF-8: {error}"))?;
        let config = serde_json::from_str::<Self>(raw)
            .map_err(|error| format!("parse file-leakage config failed: {error}"))?;
        if config.candidate_max_count == 0 {
            return Err("candidate_max_count must be greater than zero".to_string());
        }
        if config.trace_state_max_count == 0 {
            return Err("trace_state_max_count must be greater than zero".to_string());
        }
        Ok(config)
    }

    fn allowed_roots(&self, working_directory: Option<&str>) -> Result<Vec<String>, String> {
        let mut roots = self
            .additional_allowed_roots
            .iter()
            .map(|root| normalize_absolute_path(root))
            .collect::<Result<Vec<_>, _>>()?;
        if self.include_trace_working_directory {
            let working_directory = working_directory.ok_or_else(|| {
                "trace working directory is required by plugin config".to_string()
            })?;
            roots.push(normalize_absolute_path(working_directory)?);
        }
        Ok(roots)
    }
}

#[derive(serde::Serialize)]
struct FileLeakagePayload {
    residual_files: Vec<String>,
}

fn path_is_allowed(path: &str, allowed_roots: &[String]) -> bool {
    allowed_roots.iter().any(|root| {
        root == "/"
            || path == root
            || path
                .strip_prefix(root)
                .is_some_and(|suffix| suffix.starts_with('/'))
    })
}

fn normalize_observed_path(raw: &str, working_directory: Option<&str>) -> Result<String, String> {
    if raw.starts_with('/') {
        return normalize_absolute_path(raw);
    }
    let working_directory = working_directory
        .ok_or_else(|| "relative observed path requires a trace working directory".to_string())?;
    normalize_absolute_path(&format!("{working_directory}/{raw}"))
}

fn normalize_absolute_path(raw: &str) -> Result<String, String> {
    if !raw.starts_with('/') {
        return Err("all allowed roots and observed paths must be absolute".to_string());
    }
    let mut normalized = String::new();
    for component in raw.split('/') {
        match component {
            "" | "." => {}
            ".." => return Err("path traversal components are not allowed".to_string()),
            component => {
                normalized.push('/');
                normalized.push_str(component);
            }
        }
    }
    if normalized.is_empty() {
        normalized.push('/');
    }
    Ok(normalized)
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
