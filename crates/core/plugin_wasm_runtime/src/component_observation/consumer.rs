use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use plugin_system::{
    AlertHost, DEFAULT_OBSERVATION_QUEUE_CAPACITY, ObservationBatch, ObservationConsumeReport,
    ObservationConsumer, ObservationEventFamily, PluginHostGrants, PluginHostcallMetricsSource,
    PluginManifest, PluginRuntimeError, PluginRuntimeKind, PostTraceAnalyzer, PostTraceHost,
    PostTraceTask,
};
use wasmtime::component::{Component, Func};

use crate::component_observation::host::component_linker;
use crate::component_observation::wire::{
    component_call_error, observation_batch_val, parse_observation_report,
};
use crate::control::{arm_epoch_timeout, call_timeout_error, disarm_epoch_timeout};
use crate::engine::{
    PostTraceCallLimits, WasmHostcallMetrics, WasmStore, fuel_per_call, host_limits, limited_store,
    memory_max_bytes, metered_engine, reset_fuel,
};

const OBSERVATION_CONSUMER_EXPORT: &str = "actrail:plugin/observation-consumer@0.2.0";
const OBSERVATION_CONSUME_EXPORT: &str = "consume";
const POST_TRACE_ANALYZER_EXPORT: &str = "actrail:plugin/post-trace-analyzer@0.2.0";
const POST_TRACE_ANALYZE_EXPORT: &str = "analyze";
const DEFAULT_ACTION_PAGE_MAX_COUNT: u32 = 256;
const DEFAULT_ACTION_TOTAL_MAX_COUNT: u32 = 16_384;
const DEFAULT_FILE_STATE_QUERY_MAX_COUNT: u32 = 4096;

pub(crate) struct WitComponentObservationConsumer {
    instance_id: String,
    plugin_id: String,
    host_grants: Vec<String>,
    event_families: Vec<ObservationEventFamily>,
    payload_snapshot_limit: Option<usize>,
    queue_capacity: u32,
    post_trace_enabled: bool,
    post_trace_engine: wasmtime::Engine,
    hostcall_metrics: Arc<WasmHostcallMetrics>,
    state: Mutex<WitComponentObservationState>,
}

impl WitComponentObservationConsumer {
    pub(crate) fn load(
        instance_id: impl Into<String>,
        manifest: &PluginManifest,
        plugin_config: Option<&str>,
        host_grants: PluginHostGrants,
        post_trace_host: Option<Arc<dyn PostTraceHost>>,
        alert_host: Option<Arc<dyn AlertHost>>,
    ) -> Result<Self, PluginRuntimeError> {
        let post_trace_granted =
            host_grants.can_read_trace_analysis() || host_grants.can_read_trace_file_state();
        if host_grants.can_query_context()
            || host_grants.can_get_current_file_access_match()
            || host_grants.can_query_current_file_access_context()
            || host_grants.can_read_file_policy_rules()
            || host_grants.can_match_dry_run_file_policy_rules()
            || host_grants.can_validate_file_policy_rules()
            || host_grants.can_apply_file_policy_rules()
        {
            return Err(PluginRuntimeError::new(
                "wasm_runtime",
                "the requested WIT component host grants are not implemented by this runtime",
            ));
        }
        if post_trace_granted && (!manifest.has_post_trace_analyzer() || post_trace_host.is_none())
        {
            return Err(PluginRuntimeError::new(
                "wasm_runtime",
                "post-trace grants require an analyzer declaration and daemon host broker",
            ));
        }
        if host_grants.can_write_alerts() && alert_host.is_none() {
            return Err(PluginRuntimeError::new(
                "wasm_runtime",
                "alert-write grant requires the daemon alert ingress",
            ));
        }
        let artifact_path = manifest
            .selected_wasm()
            .and_then(|wasm| wasm.artifact_path.as_deref())
            .ok_or_else(|| {
                PluginRuntimeError::new(
                    "wasm_runtime",
                    "wasm plugin manifest missing [runtime.wasm]",
                )
            })?;
        let event_families = manifest.observation_event_families();
        let host_grant_values = host_grants.to_wire_values();
        let fuel_per_call = fuel_per_call(manifest);
        let memory_max_bytes = memory_max_bytes(manifest)?;
        let host_limits = host_limits(manifest)?;
        let payload_snapshot_limit = host_grants
            .can_read_payload()
            .then_some(host_limits.payload_segment_max_count);
        let queue_capacity = manifest
            .observation_queue_capacity()
            .unwrap_or(DEFAULT_OBSERVATION_QUEUE_CAPACITY);
        let hostcall_metrics = Arc::new(WasmHostcallMetrics::default());
        let engine = metered_engine()?;
        let component = Component::from_file(&engine, artifact_path).map_err(|error| {
            PluginRuntimeError::new(
                "wasm_runtime",
                format!("load wasm component artifact {artifact_path} failed: {error}"),
            )
        })?;
        let observation_export = component
            .get_export_index(None, OBSERVATION_CONSUMER_EXPORT)
            .ok_or_else(|| {
                PluginRuntimeError::new(
                    "wasm_runtime",
                    format!("wasm component missing export {OBSERVATION_CONSUMER_EXPORT}"),
                )
            })?;
        let consume_export = component
            .get_export_index(Some(&observation_export), OBSERVATION_CONSUME_EXPORT)
            .ok_or_else(|| {
                PluginRuntimeError::new(
                    "wasm_runtime",
                    format!(
                        "wasm component export {OBSERVATION_CONSUMER_EXPORT} missing {OBSERVATION_CONSUME_EXPORT}"
                    ),
                )
            })?;
        let analyze_export = if manifest.has_post_trace_analyzer() {
            let analyzer = component
                .get_export_index(None, POST_TRACE_ANALYZER_EXPORT)
                .ok_or_else(|| {
                    PluginRuntimeError::new(
                        "wasm_runtime",
                        format!("wasm component missing export {POST_TRACE_ANALYZER_EXPORT}"),
                    )
                })?;
            Some(
                component
                    .get_export_index(Some(&analyzer), POST_TRACE_ANALYZE_EXPORT)
                    .ok_or_else(|| {
                        PluginRuntimeError::new(
                            "wasm_runtime",
                            format!(
                                "wasm component export {POST_TRACE_ANALYZER_EXPORT} missing {POST_TRACE_ANALYZE_EXPORT}"
                            ),
                        )
                    })?,
            )
        } else {
            None
        };
        let mut store = limited_store(
            &engine,
            memory_max_bytes,
            host_grants,
            host_limits,
            Arc::clone(&hostcall_metrics),
        );
        store.data_mut().set_plugin_config(plugin_config);
        store.data_mut().set_post_trace_host(post_trace_host);
        store.data_mut().set_alert_host(alert_host);
        let linker = component_linker(&engine)?;
        reset_fuel(&mut store, fuel_per_call)?;
        let instance = linker
            .instantiate(&mut store, &component)
            .map_err(|error| {
                PluginRuntimeError::new(
                    "wasm_runtime",
                    format!("instantiate wasm component plugin failed: {error}"),
                )
            })?;
        let consume = instance
            .get_func(&mut store, &consume_export)
            .ok_or_else(|| {
                PluginRuntimeError::new(
                    "wasm_runtime",
                    format!(
                        "wasm component export {OBSERVATION_CONSUMER_EXPORT}.{OBSERVATION_CONSUME_EXPORT} is not a function"
                    ),
                )
            })?;
        let analyze = analyze_export
            .as_ref()
            .map(|export| {
                instance.get_func(&mut store, export).ok_or_else(|| {
                    PluginRuntimeError::new(
                        "wasm_runtime",
                        format!(
                            "wasm component export {POST_TRACE_ANALYZER_EXPORT}.{POST_TRACE_ANALYZE_EXPORT} is not a function"
                        ),
                    )
                })
            })
            .transpose()?;
        let post_trace_limits = post_trace_limits(manifest)?;
        let post_trace_enabled = analyze.is_some();

        Ok(Self {
            instance_id: instance_id.into(),
            plugin_id: manifest.id().to_string(),
            host_grants: host_grant_values,
            event_families,
            payload_snapshot_limit,
            queue_capacity,
            post_trace_enabled,
            post_trace_engine: engine.clone(),
            hostcall_metrics,
            state: Mutex::new(WitComponentObservationState {
                engine,
                store,
                consume,
                analyze,
                fuel_per_call,
                post_trace_limits,
                batch_sequence: 0,
                previous_lifecycle: None,
                deadline_generation: Arc::new(AtomicU64::new(0)),
            }),
        })
    }
}

impl ObservationConsumer for WitComponentObservationConsumer {
    fn instance_id(&self) -> &str {
        &self.instance_id
    }

    fn plugin_id(&self) -> &str {
        &self.plugin_id
    }

    fn runtime_kind(&self) -> PluginRuntimeKind {
        PluginRuntimeKind::Wasm
    }

    fn host_grants(&self) -> Vec<String> {
        self.host_grants.clone()
    }

    fn hostcall_metrics_source(&self) -> Option<Arc<dyn PluginHostcallMetricsSource>> {
        Some(self.hostcall_metrics.clone())
    }

    fn payload_snapshot_limit(&self) -> Option<usize> {
        self.payload_snapshot_limit
    }

    fn observation_queue_capacity(&self) -> u32 {
        self.queue_capacity
    }

    fn subscribed_event_families(&self) -> Vec<ObservationEventFamily> {
        self.event_families.clone()
    }

    fn post_trace_analyzer(&self) -> Option<&dyn PostTraceAnalyzer> {
        self.post_trace_enabled
            .then_some(self as &dyn PostTraceAnalyzer)
    }

    fn consume(
        &self,
        batch: ObservationBatch<'_>,
    ) -> Result<ObservationConsumeReport, PluginRuntimeError> {
        let mut state = self.state.lock().map_err(|error| {
            PluginRuntimeError::new(
                "wasm_runtime",
                format!("wasm component state lock poisoned: {error}"),
            )
        })?;
        let sequence = state.batch_sequence;
        state.batch_sequence = state
            .batch_sequence
            .checked_add(1)
            .ok_or_else(|| PluginRuntimeError::new("wasm_runtime", "batch sequence exhausted"))?;
        let lifecycle_transition = (state.previous_lifecycle != Some(batch.trace.lifecycle_state))
            .then_some(batch.trace.lifecycle_state);
        state.previous_lifecycle = Some(batch.trace.lifecycle_state);
        let input = observation_batch_val(&batch, sequence, lifecycle_transition);
        let fuel_per_call = state.fuel_per_call;
        let consume = state.consume.clone();
        reset_fuel(&mut state.store, fuel_per_call)?;
        let mut results = [wasmtime::component::Val::Result(Ok(None))];
        state.store.data_mut().set_observation_trace_context(
            batch.trace.root_working_directory.clone(),
            &batch.trace.alert_token,
        );
        state
            .store
            .data_mut()
            .set_payload_snapshot(batch.payload_segments);
        let result = consume.call(&mut state.store, &[input], &mut results);
        state.store.data_mut().clear_payload_snapshot();
        state.store.data_mut().clear_observation_trace_context();
        result.map_err(|error| component_call_error(&mut state.store, "consume", error))?;
        let parsed = parse_observation_report(
            &self.instance_id,
            self.queue_capacity,
            batch.trace.trace_id,
            results.into_iter().next().ok_or_else(|| {
                PluginRuntimeError::new("wasm_runtime", "wasm component consume returned no result")
            })?,
        );
        consume.post_return(&mut state.store).map_err(|error| {
            PluginRuntimeError::new(
                "wasm_runtime",
                format!("wasm component consume post-return failed: {error}"),
            )
        })?;
        parsed
    }
}

impl PostTraceAnalyzer for WitComponentObservationConsumer {
    fn analyze_post_trace(&self, task: PostTraceTask) -> Result<(), PluginRuntimeError> {
        let mut state = self.state.lock().map_err(|error| {
            PluginRuntimeError::new(
                "wasm_runtime",
                format!("wasm component state lock poisoned: {error}"),
            )
        })?;
        let analyze = state.analyze.clone().ok_or_else(|| {
            PluginRuntimeError::new("wasm_runtime", "post-trace analyzer export is unavailable")
        })?;
        let fuel_per_call = state.fuel_per_call;
        let limits = state.post_trace_limits;
        reset_fuel(&mut state.store, fuel_per_call)?;
        state
            .store
            .data_mut()
            .begin_post_trace_task(task.trace_id, limits);
        let input = wasmtime::component::Val::Record(vec![(
            "trace-id".to_string(),
            wasmtime::component::Val::String(task.trace_id.to_string()),
        )]);
        let mut results = [wasmtime::component::Val::Result(Ok(None))];
        let started_at = Instant::now();
        let deadline_generation = state.deadline_generation.clone();
        let deadline = arm_epoch_timeout(
            state.engine.clone(),
            &mut state.store,
            Some(task.timeout_ms),
            &deadline_generation,
        );
        let call = analyze.call(&mut state.store, &[input], &mut results);
        state.store.data_mut().end_post_trace_task();
        disarm_epoch_timeout(&mut state.store, deadline, &deadline_generation);
        if let Err(error) = call {
            return Err(call_timeout_error(
                &mut state.store,
                "wasm component post-trace analyze",
                error,
                Some(task.timeout_ms),
                started_at,
            ));
        }
        let result = parse_analyze_result(results.into_iter().next().ok_or_else(|| {
            PluginRuntimeError::new("wasm_runtime", "post-trace analyze returned no result")
        })?);
        analyze.post_return(&mut state.store).map_err(|error| {
            PluginRuntimeError::new(
                "wasm_runtime",
                format!("post-trace analyze post-return failed: {error}"),
            )
        })?;
        result
    }

    fn cancel_post_trace(&self) {
        self.post_trace_engine.increment_epoch();
    }
}

struct WitComponentObservationState {
    engine: wasmtime::Engine,
    store: WasmStore,
    consume: Func,
    analyze: Option<Func>,
    fuel_per_call: u64,
    post_trace_limits: PostTraceCallLimits,
    batch_sequence: u64,
    previous_lifecycle: Option<model_core::trace::TraceLifecycleState>,
    deadline_generation: Arc<AtomicU64>,
}

fn post_trace_limits(manifest: &PluginManifest) -> Result<PostTraceCallLimits, PluginRuntimeError> {
    Ok(PostTraceCallLimits {
        action_page_max_count: usize::try_from(
            manifest
                .hostcall_limits
                .trace_analysis
                .action_page_max_count
                .unwrap_or(DEFAULT_ACTION_PAGE_MAX_COUNT),
        )
        .map_err(limit_error("action page"))?,
        action_total_max_count: usize::try_from(
            manifest
                .hostcall_limits
                .trace_analysis
                .action_total_max_count
                .unwrap_or(DEFAULT_ACTION_TOTAL_MAX_COUNT),
        )
        .map_err(limit_error("action total"))?,
        file_state_query_max_count: usize::try_from(
            manifest
                .hostcall_limits
                .trace_file_state
                .query_max_count
                .unwrap_or(DEFAULT_FILE_STATE_QUERY_MAX_COUNT),
        )
        .map_err(limit_error("file-state query"))?,
    })
}

fn limit_error(name: &'static str) -> impl FnOnce(std::num::TryFromIntError) -> PluginRuntimeError {
    move |error| {
        PluginRuntimeError::new(
            "wasm_runtime",
            format!("{name} hostcall limit overflow: {error}"),
        )
    }
}

fn parse_analyze_result(value: wasmtime::component::Val) -> Result<(), PluginRuntimeError> {
    match value {
        wasmtime::component::Val::Result(Ok(_)) => Ok(()),
        wasmtime::component::Val::Result(Err(Some(error))) => {
            let message = match *error {
                wasmtime::component::Val::String(message) => message,
                other => format!("{other:?}"),
            };
            Err(PluginRuntimeError::new(
                "wasm_runtime",
                format!("post-trace analyzer returned error: {message}"),
            ))
        }
        wasmtime::component::Val::Result(Err(None)) => Err(PluginRuntimeError::new(
            "wasm_runtime",
            "post-trace analyzer returned error without message",
        )),
        other => Err(PluginRuntimeError::new(
            "wasm_runtime",
            format!("post-trace analyzer returned invalid result {other:?}"),
        )),
    }
}
