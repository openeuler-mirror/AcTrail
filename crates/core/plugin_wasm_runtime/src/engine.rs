use std::collections::BTreeMap;
use std::fmt::Display;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use model_core::payload::{PayloadSegment, PayloadSourceBoundary};
use plugin_system::{
    FilePolicyHost, FilePolicyReadContext, PluginHostGrants, PluginHostcallMetrics,
    PluginHostcallMetricsSource, PluginManifest, PluginPayloadReadMetrics, PluginRuntimeError,
};
use wasmtime::{Config, Engine, Store, StoreLimits, StoreLimitsBuilder};

pub(crate) const DEFAULT_WASM_FUEL_PER_CALL: u64 = 10_000_000;
pub(crate) const DEFAULT_WASM_MEMORY_MAX_BYTES: u64 = 64 * 1024 * 1024;
pub(crate) const DEFAULT_HOSTCALL_ENV_NAME_MAX_BYTES: u32 = 256;
pub(crate) const DEFAULT_HOSTCALL_ENV_VALUE_MAX_BYTES: u32 = 4096;
pub(crate) const DEFAULT_HOSTCALL_PAYLOAD_SEGMENT_MAX_COUNT: u32 = 64;
pub(crate) const DEFAULT_HOSTCALL_PAYLOAD_REF_MAX_BYTES: u32 = 256;
pub(crate) const DEFAULT_HOSTCALL_PAYLOAD_READ_MAX_BYTES: u32 = 4096;
pub(crate) const DEFAULT_HOSTCALL_CONTEXT_REF_MAX_BYTES: u32 = 128;
pub(crate) const DEFAULT_HOSTCALL_CONTEXT_QUERY_MAX_BYTES: u32 = 128;
pub(crate) const DEFAULT_HOSTCALL_CONTEXT_READ_MAX_BYTES: u32 = 1024;
pub(crate) const DEFAULT_HOSTCALL_FILE_POLICY_CONTEXT_REF_MAX_BYTES: u32 = 128;
pub(crate) const DEFAULT_HOSTCALL_FILE_POLICY_QUERY_MAX_BYTES: u32 = 128;
pub(crate) const DEFAULT_HOSTCALL_FILE_POLICY_READ_MAX_BYTES: u32 = 1024;
pub(crate) const DEFAULT_HOSTCALL_PLUGIN_CONFIG_READ_MAX_BYTES: u32 = 4096;
pub(crate) const DEFAULT_HOSTCALL_PLUGIN_COMMAND_ARGV_MAX_COUNT: u32 = 32;
pub(crate) const DEFAULT_HOSTCALL_PLUGIN_COMMAND_ARG_MAX_BYTES: u32 = 4096;
pub(crate) const DEFAULT_HOSTCALL_PLUGIN_COMMAND_OUTPUT_MAX_BYTES: u32 = 64 * 1024;
pub(crate) const DEFAULT_HOSTCALL_PLUGIN_COMMAND_TIMEOUT_MS: u64 = 30_000;

pub(crate) struct WasmStoreState {
    limits: StoreLimits,
    host_grants: PluginHostGrants,
    host_limits: WasmHostLimits,
    hostcall_metrics: Arc<WasmHostcallMetrics>,
    plugin_config: Option<Vec<u8>>,
    payload_snapshot: BTreeMap<String, PayloadSnapshotEntry>,
    control_context: Option<ControlContextSnapshot>,
    file_policy_context: Option<FilePolicyReadContext>,
    file_policy_host: Option<Arc<dyn FilePolicyHost>>,
    file_policy_owner_instance_id: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct WasmHostLimits {
    pub(crate) env_name_max_bytes: usize,
    pub(crate) env_value_max_bytes: usize,
    pub(crate) payload_segment_max_count: usize,
    pub(crate) payload_ref_max_bytes: usize,
    pub(crate) payload_read_max_bytes: usize,
    pub(crate) context_ref_max_bytes: usize,
    pub(crate) context_query_max_bytes: usize,
    pub(crate) context_read_max_bytes: usize,
    pub(crate) file_policy_context_ref_max_bytes: usize,
    pub(crate) file_policy_query_max_bytes: usize,
    pub(crate) file_policy_io_max_bytes: usize,
    pub(crate) plugin_config_read_max_bytes: usize,
    pub(crate) plugin_command_argv_max_count: usize,
    pub(crate) plugin_command_arg_max_bytes: usize,
    pub(crate) plugin_command_output_max_bytes: usize,
    pub(crate) plugin_command_timeout_ms: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct ControlContextSnapshot {
    pub(crate) context_ref: String,
    pub(crate) decision_id: String,
    pub(crate) trace_id: String,
    pub(crate) subject: String,
    pub(crate) operation: String,
    pub(crate) target_summary: String,
    pub(crate) actor_process_identity: String,
}

pub(crate) struct PayloadSnapshotEntry {
    pub(crate) source_boundary: PayloadSourceBoundary,
    pub(crate) bytes: Option<Vec<u8>>,
}

#[derive(Debug, Default)]
pub(crate) struct WasmHostcallMetrics {
    payload_read_calls: AtomicU64,
    payload_read_bytes: AtomicU64,
    payload_read_denied: AtomicU64,
    payload_read_not_found: AtomicU64,
    payload_read_invalid: AtomicU64,
    payload_read_too_large: AtomicU64,
    payload_read_truncated: AtomicU64,
    payload_read_latency_total_ns: AtomicU64,
    payload_read_latency_max_ns: AtomicU64,
}

pub(crate) type WasmStore = Store<WasmStoreState>;

pub(crate) fn fuel_per_call(manifest: &PluginManifest) -> u64 {
    manifest
        .selected_wasm()
        .and_then(|wasm| wasm.resources.fuel_per_call)
        .unwrap_or(DEFAULT_WASM_FUEL_PER_CALL)
}

pub(crate) fn memory_max_bytes(manifest: &PluginManifest) -> Result<usize, PluginRuntimeError> {
    let limit = manifest
        .selected_wasm()
        .and_then(|wasm| wasm.resources.memory_max_bytes)
        .unwrap_or(DEFAULT_WASM_MEMORY_MAX_BYTES);
    usize::try_from(limit).map_err(|error| {
        PluginRuntimeError::new(
            "wasm_runtime",
            format!("wasm memory max bytes overflow: {error}"),
        )
    })
}

pub(crate) fn host_limits(manifest: &PluginManifest) -> Result<WasmHostLimits, PluginRuntimeError> {
    let env_name_max_bytes = manifest
        .hostcall_limits
        .env
        .name_max_bytes
        .unwrap_or(DEFAULT_HOSTCALL_ENV_NAME_MAX_BYTES);
    let env_value_max_bytes = manifest
        .hostcall_limits
        .env
        .value_max_bytes
        .unwrap_or(DEFAULT_HOSTCALL_ENV_VALUE_MAX_BYTES);
    let payload_segment_max_count = manifest
        .hostcall_limits
        .payload
        .segment_max_count
        .unwrap_or(DEFAULT_HOSTCALL_PAYLOAD_SEGMENT_MAX_COUNT);
    let payload_ref_max_bytes = manifest
        .hostcall_limits
        .payload
        .ref_max_bytes
        .unwrap_or(DEFAULT_HOSTCALL_PAYLOAD_REF_MAX_BYTES);
    let payload_read_max_bytes = manifest
        .hostcall_limits
        .payload
        .read_max_bytes
        .unwrap_or(DEFAULT_HOSTCALL_PAYLOAD_READ_MAX_BYTES);
    let context_ref_max_bytes = manifest
        .hostcall_limits
        .context
        .ref_max_bytes
        .unwrap_or(DEFAULT_HOSTCALL_CONTEXT_REF_MAX_BYTES);
    let context_query_max_bytes = manifest
        .hostcall_limits
        .context
        .query_max_bytes
        .unwrap_or(DEFAULT_HOSTCALL_CONTEXT_QUERY_MAX_BYTES);
    let context_read_max_bytes = manifest
        .hostcall_limits
        .context
        .read_max_bytes
        .unwrap_or(DEFAULT_HOSTCALL_CONTEXT_READ_MAX_BYTES);
    let file_policy_context_ref_max_bytes = manifest
        .hostcall_limits
        .file_policy
        .context_ref_max_bytes
        .unwrap_or(DEFAULT_HOSTCALL_FILE_POLICY_CONTEXT_REF_MAX_BYTES);
    let file_policy_query_max_bytes = manifest
        .hostcall_limits
        .file_policy
        .query_max_bytes
        .unwrap_or(DEFAULT_HOSTCALL_FILE_POLICY_QUERY_MAX_BYTES);
    let file_policy_io_max_bytes = manifest
        .hostcall_limits
        .file_policy
        .read_max_bytes
        .unwrap_or(DEFAULT_HOSTCALL_FILE_POLICY_READ_MAX_BYTES);
    let plugin_config_read_max_bytes = manifest
        .hostcall_limits
        .plugin_config
        .read_max_bytes
        .unwrap_or(DEFAULT_HOSTCALL_PLUGIN_CONFIG_READ_MAX_BYTES);
    let plugin_command_argv_max_count = manifest
        .hostcall_limits
        .plugin_command
        .argv_max_count
        .unwrap_or(DEFAULT_HOSTCALL_PLUGIN_COMMAND_ARGV_MAX_COUNT);
    let plugin_command_arg_max_bytes = manifest
        .hostcall_limits
        .plugin_command
        .arg_max_bytes
        .unwrap_or(DEFAULT_HOSTCALL_PLUGIN_COMMAND_ARG_MAX_BYTES);
    let plugin_command_output_max_bytes = manifest
        .hostcall_limits
        .plugin_command
        .output_max_bytes
        .unwrap_or(DEFAULT_HOSTCALL_PLUGIN_COMMAND_OUTPUT_MAX_BYTES);
    let plugin_command_timeout_ms = manifest
        .hostcall_limits
        .plugin_command
        .timeout_ms
        .unwrap_or(DEFAULT_HOSTCALL_PLUGIN_COMMAND_TIMEOUT_MS);
    Ok(WasmHostLimits {
        env_name_max_bytes: usize::try_from(env_name_max_bytes).map_err(|error| {
            PluginRuntimeError::new(
                "wasm_runtime",
                format!("env name hostcall byte limit overflow: {error}"),
            )
        })?,
        env_value_max_bytes: usize::try_from(env_value_max_bytes).map_err(|error| {
            PluginRuntimeError::new(
                "wasm_runtime",
                format!("env value hostcall byte limit overflow: {error}"),
            )
        })?,
        payload_segment_max_count: usize::try_from(payload_segment_max_count).map_err(|error| {
            PluginRuntimeError::new(
                "wasm_runtime",
                format!("payload segment snapshot count limit overflow: {error}"),
            )
        })?,
        payload_ref_max_bytes: usize::try_from(payload_ref_max_bytes).map_err(|error| {
            PluginRuntimeError::new(
                "wasm_runtime",
                format!("payload ref hostcall byte limit overflow: {error}"),
            )
        })?,
        payload_read_max_bytes: usize::try_from(payload_read_max_bytes).map_err(|error| {
            PluginRuntimeError::new(
                "wasm_runtime",
                format!("payload read hostcall byte limit overflow: {error}"),
            )
        })?,
        context_ref_max_bytes: usize::try_from(context_ref_max_bytes).map_err(|error| {
            PluginRuntimeError::new(
                "wasm_runtime",
                format!("context-ref hostcall byte limit overflow: {error}"),
            )
        })?,
        context_query_max_bytes: usize::try_from(context_query_max_bytes).map_err(|error| {
            PluginRuntimeError::new(
                "wasm_runtime",
                format!("context query hostcall byte limit overflow: {error}"),
            )
        })?,
        context_read_max_bytes: usize::try_from(context_read_max_bytes).map_err(|error| {
            PluginRuntimeError::new(
                "wasm_runtime",
                format!("context read hostcall byte limit overflow: {error}"),
            )
        })?,
        file_policy_context_ref_max_bytes: usize::try_from(file_policy_context_ref_max_bytes)
            .map_err(|error| {
                PluginRuntimeError::new(
                    "wasm_runtime",
                    format!("file-policy context-ref hostcall byte limit overflow: {error}"),
                )
            })?,
        file_policy_query_max_bytes: usize::try_from(file_policy_query_max_bytes).map_err(
            |error| {
                PluginRuntimeError::new(
                    "wasm_runtime",
                    format!("file-policy query hostcall byte limit overflow: {error}"),
                )
            },
        )?,
        file_policy_io_max_bytes: usize::try_from(file_policy_io_max_bytes).map_err(|error| {
            PluginRuntimeError::new(
                "wasm_runtime",
                format!("file-policy read hostcall byte limit overflow: {error}"),
            )
        })?,
        plugin_config_read_max_bytes: usize::try_from(plugin_config_read_max_bytes).map_err(
            |error| {
                PluginRuntimeError::new(
                    "wasm_runtime",
                    format!("plugin config read hostcall byte limit overflow: {error}"),
                )
            },
        )?,
        plugin_command_argv_max_count: usize::try_from(plugin_command_argv_max_count).map_err(
            |error| {
                PluginRuntimeError::new(
                    "wasm_runtime",
                    format!("plugin command argv count limit overflow: {error}"),
                )
            },
        )?,
        plugin_command_arg_max_bytes: usize::try_from(plugin_command_arg_max_bytes).map_err(
            |error| {
                PluginRuntimeError::new(
                    "wasm_runtime",
                    format!("plugin command arg byte limit overflow: {error}"),
                )
            },
        )?,
        plugin_command_output_max_bytes: usize::try_from(plugin_command_output_max_bytes).map_err(
            |error| {
                PluginRuntimeError::new(
                    "wasm_runtime",
                    format!("plugin command output byte limit overflow: {error}"),
                )
            },
        )?,
        plugin_command_timeout_ms,
    })
}

pub(crate) fn metered_engine() -> Result<Engine, PluginRuntimeError> {
    let mut config = Config::new();
    config.consume_fuel(true);
    config.epoch_interruption(true);
    config.wasm_component_model(true);
    Engine::new(&config).map_err(|error| {
        PluginRuntimeError::new(
            "wasm_runtime",
            format!("create wasm engine failed: {error}"),
        )
    })
}

impl WasmStoreState {
    pub(crate) fn host_grants(&self) -> &PluginHostGrants {
        &self.host_grants
    }

    pub(crate) fn host_limits(&self) -> &WasmHostLimits {
        &self.host_limits
    }

    pub(crate) fn hostcall_metrics(&self) -> &WasmHostcallMetrics {
        &self.hostcall_metrics
    }

    pub(crate) fn payload_entry(&self, ref_id: &str) -> Option<&PayloadSnapshotEntry> {
        self.payload_snapshot.get(ref_id)
    }

    pub(crate) fn plugin_config(&self) -> Option<&[u8]> {
        self.plugin_config.as_deref()
    }

    pub(crate) fn set_plugin_config(&mut self, plugin_config: Option<&str>) {
        self.plugin_config = plugin_config.map(|config| config.as_bytes().to_vec());
    }

    pub(crate) fn set_payload_snapshot(&mut self, segments: &[PayloadSegment]) {
        self.payload_snapshot.clear();
        for segment in segments
            .iter()
            .take(self.host_limits.payload_segment_max_count)
        {
            self.payload_snapshot.insert(
                segment.segment_id.to_string(),
                PayloadSnapshotEntry {
                    source_boundary: segment.source_boundary,
                    bytes: self
                        .host_grants
                        .can_read_payload_source(segment.source_boundary)
                        .then(|| segment.bytes.clone()),
                },
            );
        }
    }

    pub(crate) fn clear_payload_snapshot(&mut self) {
        self.payload_snapshot.clear();
    }

    pub(crate) fn control_context(&self) -> Option<&ControlContextSnapshot> {
        self.control_context.as_ref()
    }

    pub(crate) fn set_control_context(&mut self, context: Option<ControlContextSnapshot>) {
        self.control_context = context;
    }

    pub(crate) fn clear_control_context(&mut self) {
        self.control_context = None;
    }

    pub(crate) fn file_policy_context(&self) -> Option<&FilePolicyReadContext> {
        self.file_policy_context.as_ref()
    }

    pub(crate) fn set_file_policy_context(&mut self, context: Option<FilePolicyReadContext>) {
        self.file_policy_context = context;
    }

    pub(crate) fn clear_file_policy_context(&mut self) {
        self.file_policy_context = None;
    }

    pub(crate) fn file_policy_host(&self) -> Option<&Arc<dyn FilePolicyHost>> {
        self.file_policy_host.as_ref()
    }

    pub(crate) fn file_policy_owner_instance_id(&self) -> Option<&str> {
        self.file_policy_owner_instance_id.as_deref()
    }

    pub(crate) fn set_file_policy_host(
        &mut self,
        owner_instance_id: impl Into<String>,
        host: Option<Arc<dyn FilePolicyHost>>,
    ) {
        self.file_policy_owner_instance_id = host.as_ref().map(|_| owner_instance_id.into());
        self.file_policy_host = host;
    }
}

impl WasmHostcallMetrics {
    pub(crate) fn record_payload_read(
        &self,
        result: i64,
        bytes: u64,
        truncated: bool,
        latency: Duration,
    ) {
        self.payload_read_calls.fetch_add(1, Ordering::Relaxed);
        self.payload_read_bytes.fetch_add(bytes, Ordering::Relaxed);
        match result {
            -1 => {
                self.payload_read_denied.fetch_add(1, Ordering::Relaxed);
            }
            -2 => {
                self.payload_read_not_found.fetch_add(1, Ordering::Relaxed);
            }
            -3 => {
                self.payload_read_invalid.fetch_add(1, Ordering::Relaxed);
            }
            -4 => {
                self.payload_read_too_large.fetch_add(1, Ordering::Relaxed);
            }
            _ => {}
        }
        if truncated {
            self.payload_read_truncated.fetch_add(1, Ordering::Relaxed);
        }
        let latency_ns = saturating_u64(latency.as_nanos());
        self.payload_read_latency_total_ns
            .fetch_add(latency_ns, Ordering::Relaxed);
        update_max(&self.payload_read_latency_max_ns, latency_ns);
    }
}

impl PluginHostcallMetricsSource for WasmHostcallMetrics {
    fn snapshot(&self) -> PluginHostcallMetrics {
        PluginHostcallMetrics {
            payload_read: PluginPayloadReadMetrics {
                calls: self.payload_read_calls.load(Ordering::Relaxed),
                bytes: self.payload_read_bytes.load(Ordering::Relaxed),
                denied: self.payload_read_denied.load(Ordering::Relaxed),
                not_found: self.payload_read_not_found.load(Ordering::Relaxed),
                invalid: self.payload_read_invalid.load(Ordering::Relaxed),
                too_large: self.payload_read_too_large.load(Ordering::Relaxed),
                truncated: self.payload_read_truncated.load(Ordering::Relaxed),
                latency_total_ns: self.payload_read_latency_total_ns.load(Ordering::Relaxed),
                latency_max_ns: self.payload_read_latency_max_ns.load(Ordering::Relaxed),
            },
        }
    }
}

pub(crate) fn limited_store(
    engine: &Engine,
    memory_max_bytes: usize,
    host_grants: PluginHostGrants,
    host_limits: WasmHostLimits,
    hostcall_metrics: Arc<WasmHostcallMetrics>,
) -> WasmStore {
    let limits = StoreLimitsBuilder::new()
        .memory_size(memory_max_bytes)
        .build();
    let mut store = Store::new(
        engine,
        WasmStoreState {
            limits,
            host_grants,
            host_limits,
            hostcall_metrics,
            plugin_config: None,
            payload_snapshot: BTreeMap::new(),
            control_context: None,
            file_policy_context: None,
            file_policy_host: None,
            file_policy_owner_instance_id: None,
        },
    );
    store.limiter(|state| &mut state.limits);
    store.epoch_deadline_trap();
    reset_epoch_deadline_unbounded(&mut store);
    store
}

fn saturating_u64(value: u128) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

fn update_max(target: &AtomicU64, value: u64) {
    let mut current = target.load(Ordering::Relaxed);
    while value > current {
        match target.compare_exchange_weak(current, value, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return,
            Err(next) => current = next,
        }
    }
}

pub(crate) fn reset_epoch_deadline_unbounded(store: &mut WasmStore) {
    store.set_epoch_deadline(u64::MAX / 2);
}

pub(crate) fn reset_fuel(
    store: &mut WasmStore,
    fuel_per_call: u64,
) -> Result<(), PluginRuntimeError> {
    store.set_fuel(fuel_per_call).map_err(|error| {
        PluginRuntimeError::new("wasm_runtime", format!("set wasm fuel failed: {error}"))
    })
}

pub(crate) fn call_error(
    store: &mut WasmStore,
    operation: &str,
    error: impl Display,
) -> PluginRuntimeError {
    if store.get_fuel().map(|fuel| fuel == 0).unwrap_or(false) {
        PluginRuntimeError::new(
            "wasm_runtime",
            format!("wasm fuel exhausted during {operation}: {error}"),
        )
    } else {
        PluginRuntimeError::new("wasm_runtime", format!("{operation} trapped: {error}"))
    }
}
