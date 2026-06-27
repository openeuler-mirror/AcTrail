use std::sync::Arc;
use std::sync::Mutex;

use plugin_system::{
    DEFAULT_OBSERVATION_QUEUE_CAPACITY, ObservationBatch, ObservationConsumeReport,
    ObservationConsumer, ObservationEventFamily, PluginHostGrants, PluginHostcallMetricsSource,
    PluginManifest, PluginRuntimeError, PluginRuntimeKind, PluginWasmAbi,
};
use serde_json::json;
use wasmtime::{Memory, Module, TypedFunc};

use crate::component_observation::WitComponentObservationConsumer;
use crate::engine::{
    WasmHostcallMetrics, WasmStore, call_error, fuel_per_call, host_limits, limited_store,
    memory_max_bytes, metered_engine, reset_fuel,
};
use crate::host::host_linker;
use crate::memory::write_guest_bytes;

pub fn build_wasm_observation_consumer(
    instance_id: impl Into<String>,
    manifest: &PluginManifest,
    plugin_config: Option<&str>,
    host_grants: PluginHostGrants,
) -> Result<WasmObservationConsumer, PluginRuntimeError> {
    let wasm = manifest.selected_wasm().ok_or_else(|| {
        PluginRuntimeError::new(
            "wasm_runtime",
            "wasm plugin manifest missing [runtime.wasm]",
        )
    })?;
    match wasm.abi {
        PluginWasmAbi::LegacyModule => {
            let consumer = LegacyWasmObservationConsumer::load(
                instance_id,
                manifest,
                plugin_config.unwrap_or_default(),
                host_grants,
            )?;
            Ok(WasmObservationConsumer::new(Box::new(consumer)))
        }
        PluginWasmAbi::WitComponent => {
            let consumer = WitComponentObservationConsumer::load(
                instance_id,
                manifest,
                plugin_config,
                host_grants,
            )?;
            Ok(WasmObservationConsumer::new(Box::new(consumer)))
        }
    }
}

pub struct WasmObservationConsumer {
    inner: Box<dyn ObservationConsumer>,
}

impl WasmObservationConsumer {
    fn new(inner: Box<dyn ObservationConsumer>) -> Self {
        Self { inner }
    }
}

impl ObservationConsumer for WasmObservationConsumer {
    fn instance_id(&self) -> &str {
        self.inner.instance_id()
    }

    fn plugin_id(&self) -> &str {
        self.inner.plugin_id()
    }

    fn runtime_kind(&self) -> PluginRuntimeKind {
        self.inner.runtime_kind()
    }

    fn host_grants(&self) -> Vec<String> {
        self.inner.host_grants()
    }

    fn hostcall_metrics_source(&self) -> Option<Arc<dyn PluginHostcallMetricsSource>> {
        self.inner.hostcall_metrics_source()
    }

    fn payload_snapshot_limit(&self) -> Option<usize> {
        self.inner.payload_snapshot_limit()
    }

    fn observation_queue_capacity(&self) -> u32 {
        self.inner.observation_queue_capacity()
    }

    fn subscribed_event_families(&self) -> Vec<ObservationEventFamily> {
        self.inner.subscribed_event_families()
    }

    fn consume(
        &self,
        batch: ObservationBatch<'_>,
    ) -> Result<ObservationConsumeReport, PluginRuntimeError> {
        self.inner.consume(batch)
    }
}

struct LegacyWasmObservationConsumer {
    instance_id: String,
    plugin_id: String,
    host_grants: Vec<String>,
    event_families: Vec<ObservationEventFamily>,
    payload_snapshot_limit: Option<usize>,
    queue_capacity: u32,
    hostcall_metrics: Arc<WasmHostcallMetrics>,
    state: Mutex<WasmObservationState>,
}

impl LegacyWasmObservationConsumer {
    fn load(
        instance_id: impl Into<String>,
        manifest: &PluginManifest,
        plugin_config: &str,
        host_grants: PluginHostGrants,
    ) -> Result<Self, PluginRuntimeError> {
        let artifact_path = manifest
            .selected_wasm()
            .and_then(|wasm| wasm.artifact_path.as_deref())
            .ok_or_else(|| {
                PluginRuntimeError::new(
                    "wasm_runtime",
                    "wasm plugin manifest missing [runtime.wasm]",
                )
            })?;
        let host_grant_values = host_grants.to_wire_values();
        let event_families = manifest.observation_event_families();
        let fuel_per_call = fuel_per_call(manifest);
        let memory_max_bytes = memory_max_bytes(manifest)?;
        let host_limits = host_limits(manifest)?;
        let payload_snapshot_limit = if host_grants.can_read_payload() {
            Some(host_limits.payload_segment_max_count)
        } else {
            None
        };
        let queue_capacity = manifest
            .observation_queue_capacity()
            .unwrap_or(DEFAULT_OBSERVATION_QUEUE_CAPACITY);
        let hostcall_metrics = Arc::new(WasmHostcallMetrics::default());
        let engine = metered_engine()?;
        let module = Module::from_file(&engine, artifact_path).map_err(|error| {
            PluginRuntimeError::new(
                "wasm_runtime",
                format!("load wasm artifact {artifact_path} failed: {error}"),
            )
        })?;
        let mut store = limited_store(
            &engine,
            memory_max_bytes,
            host_grants,
            host_limits,
            Arc::clone(&hostcall_metrics),
        );
        let linker = host_linker(&engine)?;
        let instance = linker.instantiate(&mut store, &module).map_err(|error| {
            PluginRuntimeError::new(
                "wasm_runtime",
                format!("instantiate wasm plugin failed: {error}"),
            )
        })?;
        let memory = instance.get_memory(&mut store, "memory").ok_or_else(|| {
            PluginRuntimeError::new("wasm_runtime", "wasm plugin missing memory export")
        })?;
        let alloc = instance
            .get_typed_func::<i32, i32>(&mut store, "actrail_alloc")
            .map_err(|error| {
                PluginRuntimeError::new(
                    "wasm_runtime",
                    format!("wasm plugin missing actrail_alloc: {error}"),
                )
            })?;
        let consume = instance
            .get_typed_func::<(i32, i32), i64>(&mut store, "actrail_observation_consume")
            .map_err(|error| {
                PluginRuntimeError::new(
                    "wasm_runtime",
                    format!("wasm plugin missing actrail_observation_consume: {error}"),
                )
            })?;
        let init = instance
            .get_typed_func::<(i32, i32), i32>(&mut store, "actrail_plugin_init")
            .ok();

        if let Some(init) = init {
            reset_fuel(&mut store, fuel_per_call)?;
            let (ptr, len) =
                write_guest_bytes(&mut store, memory, alloc.clone(), plugin_config.as_bytes())?;
            let status = init
                .call(&mut store, (ptr, len))
                .map_err(|error| call_error(&mut store, "wasm plugin init", error))?;
            if status != 0 {
                return Err(PluginRuntimeError::new(
                    "wasm_runtime",
                    format!("wasm plugin init returned {status}"),
                ));
            }
        }

        Ok(Self {
            instance_id: instance_id.into(),
            plugin_id: manifest.id().to_string(),
            host_grants: host_grant_values,
            event_families,
            payload_snapshot_limit,
            queue_capacity,
            hostcall_metrics,
            state: Mutex::new(WasmObservationState {
                store,
                memory,
                alloc,
                consume,
                fuel_per_call,
            }),
        })
    }
}

impl ObservationConsumer for LegacyWasmObservationConsumer {
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

    fn consume(
        &self,
        batch: ObservationBatch<'_>,
    ) -> Result<ObservationConsumeReport, PluginRuntimeError> {
        let envelope = observation_envelope(&batch)?;
        let mut state = self.state.lock().map_err(|error| {
            PluginRuntimeError::new("wasm_runtime", format!("wasm state lock poisoned: {error}"))
        })?;
        let memory = state.memory;
        let alloc = state.alloc.clone();
        let fuel_per_call = state.fuel_per_call;
        reset_fuel(&mut state.store, fuel_per_call)?;
        state
            .store
            .data_mut()
            .set_payload_snapshot(batch.payload_segments);
        let (ptr, len) = write_guest_bytes(&mut state.store, memory, alloc, envelope.as_bytes())?;
        let consume = state.consume.clone();
        let result = consume.call(&mut state.store, (ptr, len));
        state.store.data_mut().clear_payload_snapshot();
        let consumed = result
            .map_err(|error| call_error(&mut state.store, "wasm observation consume", error))?;
        if consumed < 0 {
            return Err(PluginRuntimeError::new(
                "wasm_runtime",
                format!("wasm observation consume returned {consumed}"),
            ));
        }
        Ok(ObservationConsumeReport::empty())
    }
}

struct WasmObservationState {
    store: WasmStore,
    memory: Memory,
    alloc: TypedFunc<i32, i32>,
    consume: TypedFunc<(i32, i32), i64>,
    fuel_per_call: u64,
}

fn observation_envelope(batch: &ObservationBatch<'_>) -> Result<String, PluginRuntimeError> {
    serde_json::to_string(&json!({
        "schema_version": "actrail.observation.v0",
        "trace_id": batch.trace.trace_id.to_string(),
        "semantic_action_count": batch.semantic_actions.len(),
        "semantic_link_count": batch.semantic_links.len(),
        "payload_refs": batch.payload_segments.iter().map(|segment| {
            json!({
                "id": segment.segment_id.to_string(),
                "trace_id": segment.trace_id.to_string(),
                "captured_size": segment.captured_size,
                "original_size": segment.original_size,
                "redaction": format!("{:?}", segment.redaction),
                "truncation": format!("{:?}", segment.truncation),
            })
        }).collect::<Vec<_>>(),
        "actions": batch.semantic_actions.iter().map(|action| {
            json!({
                "action_id": action.action_id,
                "kind": format!("{:?}", action.kind),
                "status": format!("{:?}", action.status),
                "title": action.title,
            })
        }).collect::<Vec<_>>(),
    }))
    .map_err(|error| {
        PluginRuntimeError::new(
            "wasm_runtime",
            format!("encode observation envelope failed: {error}"),
        )
    })
}
