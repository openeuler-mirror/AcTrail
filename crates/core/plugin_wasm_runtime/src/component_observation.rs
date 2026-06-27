use std::sync::{Arc, Mutex};
use std::time::Instant;

use model_core::ids::TraceId;
use plugin_system::{
    DEFAULT_OBSERVATION_QUEUE_CAPACITY, ObservationBatch, ObservationConsumeReport,
    ObservationConsumer, ObservationEventFamily, PluginDroppedRecord, PluginHostGrants,
    PluginHostcallMetricsSource, PluginManifest, PluginRuntimeError, PluginRuntimeKind,
};
use semantic_action::SemanticAction;
use wasmtime::Engine;
use wasmtime::component::{Component, Func, Linker as ComponentLinker, Val};

use crate::engine::{
    WasmHostcallMetrics, WasmStore, WasmStoreState, fuel_per_call, host_limits, limited_store,
    memory_max_bytes, metered_engine, reset_fuel,
};
use crate::host::component_read_config;

const OBSERVATION_CONSUMER_EXPORT: &str = "actrail:plugin/observation-consumer@0.1.0";
const OBSERVATION_CONSUME_EXPORT: &str = "consume";
const HOST_IMPORT: &str = "actrail:plugin/host@0.1.0";
const READ_CONFIG_IMPORT: &str = "read-config";
const ENV_READ_IMPORT: &str = "env-read";
const READ_PAYLOAD_IMPORT: &str = "read-payload";
const COMPONENT_ENV_READ_DENIED: &str = "denied";
const COMPONENT_ENV_READ_INVALID: &str = "invalid";
const COMPONENT_ENV_READ_NOT_FOUND: &str = "not-found";
const COMPONENT_ENV_READ_TOO_LARGE: &str = "too-large";
const COMPONENT_PAYLOAD_READ_OK: &str = "ok";
const COMPONENT_PAYLOAD_READ_DENIED: &str = "denied";
const COMPONENT_PAYLOAD_READ_NOT_FOUND: &str = "not-found";
const COMPONENT_PAYLOAD_READ_TRUNCATED: &str = "truncated";
const PAYLOAD_READ_DENIED: i64 = -1;
const PAYLOAD_READ_NOT_FOUND: i64 = -2;
const PAYLOAD_READ_INVALID: i64 = -3;
const PAYLOAD_READ_TOO_LARGE: i64 = -4;

pub(crate) struct WitComponentObservationConsumer {
    instance_id: String,
    plugin_id: String,
    host_grants: Vec<String>,
    event_families: Vec<ObservationEventFamily>,
    payload_snapshot_limit: Option<usize>,
    queue_capacity: u32,
    hostcall_metrics: Arc<WasmHostcallMetrics>,
    state: Mutex<WitComponentObservationState>,
}

impl WitComponentObservationConsumer {
    pub(crate) fn load(
        instance_id: impl Into<String>,
        manifest: &PluginManifest,
        plugin_config: Option<&str>,
        host_grants: PluginHostGrants,
    ) -> Result<Self, PluginRuntimeError> {
        if host_grants.can_query_context() || host_grants.can_read_file_policy() {
            return Err(PluginRuntimeError::new(
                "wasm_runtime",
                "only env-read and payload-read grants are implemented for WIT component plugins",
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
        let mut store = limited_store(
            &engine,
            memory_max_bytes,
            host_grants,
            host_limits,
            Arc::clone(&hostcall_metrics),
        );
        store.data_mut().set_plugin_config(plugin_config);
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

        Ok(Self {
            instance_id: instance_id.into(),
            plugin_id: manifest.id().to_string(),
            host_grants: host_grant_values,
            event_families,
            payload_snapshot_limit,
            queue_capacity,
            hostcall_metrics,
            state: Mutex::new(WitComponentObservationState {
                store,
                consume,
                fuel_per_call,
            }),
        })
    }
}

fn component_linker(
    engine: &Engine,
) -> Result<ComponentLinker<WasmStoreState>, PluginRuntimeError> {
    let mut linker = ComponentLinker::new(engine);
    let mut host = linker.instance(HOST_IMPORT).map_err(|error| {
        PluginRuntimeError::new(
            "wasm_runtime",
            format!("define wasm component host instance failed: {error}"),
        )
    })?;
    host.func_new(READ_CONFIG_IMPORT, |store, _ty, params, results| {
        component_read_config(store, params, results);
        Ok(())
    })
    .map_err(|error| {
        PluginRuntimeError::new(
            "wasm_runtime",
            format!("define wasm component read-config host import failed: {error}"),
        )
    })?;
    host.func_new(ENV_READ_IMPORT, |store, _ty, params, results| {
        component_env_read(store, params, results);
        Ok(())
    })
    .map_err(|error| {
        PluginRuntimeError::new(
            "wasm_runtime",
            format!("define wasm component env-read host import failed: {error}"),
        )
    })?;
    host.func_new(READ_PAYLOAD_IMPORT, |store, _ty, params, results| {
        component_read_payload(store, params, results);
        Ok(())
    })
    .map_err(|error| {
        PluginRuntimeError::new(
            "wasm_runtime",
            format!("define wasm component read-payload host import failed: {error}"),
        )
    })?;
    Ok(linker)
}

fn component_read_payload(
    store: wasmtime::StoreContextMut<'_, WasmStoreState>,
    params: &[Val],
    results: &mut [Val],
) {
    let started_at = Instant::now();
    let outcome = component_read_payload_inner(store.data(), params);
    store.data().hostcall_metrics().record_payload_read(
        outcome.metric_result,
        outcome.metric_bytes,
        outcome.truncated,
        started_at.elapsed(),
    );
    set_component_payload_chunk(results, outcome);
}

fn component_read_payload_inner(
    state: &WasmStoreState,
    params: &[Val],
) -> ComponentPayloadReadOutcome {
    if !state.host_grants().can_read_payload() {
        return ComponentPayloadReadOutcome::error(
            COMPONENT_PAYLOAD_READ_DENIED,
            PAYLOAD_READ_DENIED,
        );
    }
    let [
        Val::Record(ref_fields),
        Val::U64(offset),
        Val::U64(max_bytes),
    ] = params
    else {
        return ComponentPayloadReadOutcome::error(
            COMPONENT_PAYLOAD_READ_DENIED,
            PAYLOAD_READ_INVALID,
        );
    };
    let Some(ref_id) = component_record_string(ref_fields, "id") else {
        return ComponentPayloadReadOutcome::error(
            COMPONENT_PAYLOAD_READ_DENIED,
            PAYLOAD_READ_INVALID,
        );
    };
    if ref_id.len() > state.host_limits().payload_ref_max_bytes {
        return ComponentPayloadReadOutcome::error(
            COMPONENT_PAYLOAD_READ_DENIED,
            PAYLOAD_READ_TOO_LARGE,
        );
    }
    let max_bytes = match usize::try_from(*max_bytes) {
        Ok(max_bytes) => max_bytes,
        Err(_) => {
            return ComponentPayloadReadOutcome::error(
                COMPONENT_PAYLOAD_READ_DENIED,
                PAYLOAD_READ_TOO_LARGE,
            );
        }
    };
    if max_bytes > state.host_limits().payload_read_max_bytes {
        return ComponentPayloadReadOutcome::error(
            COMPONENT_PAYLOAD_READ_DENIED,
            PAYLOAD_READ_TOO_LARGE,
        );
    }
    let Some(payload) = state.payload_entry(ref_id) else {
        return ComponentPayloadReadOutcome::error(
            COMPONENT_PAYLOAD_READ_NOT_FOUND,
            PAYLOAD_READ_NOT_FOUND,
        );
    };
    if !state
        .host_grants()
        .can_read_payload_source(payload.source_boundary)
    {
        return ComponentPayloadReadOutcome::error(
            COMPONENT_PAYLOAD_READ_DENIED,
            PAYLOAD_READ_DENIED,
        );
    }
    let Some(bytes) = payload.bytes.as_deref() else {
        return ComponentPayloadReadOutcome::error(
            COMPONENT_PAYLOAD_READ_NOT_FOUND,
            PAYLOAD_READ_NOT_FOUND,
        );
    };
    let offset_usize = usize::try_from(*offset).unwrap_or(usize::MAX);
    if offset_usize >= bytes.len() {
        return ComponentPayloadReadOutcome::success(
            COMPONENT_PAYLOAD_READ_OK,
            Vec::new(),
            *offset,
            None,
            Some(bytes.len()),
            false,
        );
    }
    let available = &bytes[offset_usize..];
    let count = available.len().min(max_bytes);
    let truncated = available.len() > count;
    let chunk = available[..count].to_vec();
    let next_offset = if truncated {
        offset.checked_add(u64::try_from(count).unwrap_or(u64::MAX))
    } else {
        None
    };
    let status = if truncated {
        COMPONENT_PAYLOAD_READ_TRUNCATED
    } else {
        COMPONENT_PAYLOAD_READ_OK
    };
    ComponentPayloadReadOutcome::success(
        status,
        chunk,
        *offset,
        next_offset,
        Some(bytes.len()),
        truncated,
    )
}

fn component_record_string<'a>(fields: &'a [(String, Val)], name: &str) -> Option<&'a str> {
    fields.iter().find_map(|(field, value)| {
        if field == name
            && let Val::String(value) = value
        {
            Some(value.as_str())
        } else {
            None
        }
    })
}

#[derive(Debug)]
struct ComponentPayloadReadOutcome {
    status: &'static str,
    bytes: Vec<u8>,
    offset: u64,
    next_offset: Option<u64>,
    total_size_hint: Option<u64>,
    truncated: bool,
    metric_result: i64,
    metric_bytes: u64,
}

impl ComponentPayloadReadOutcome {
    fn error(status: &'static str, metric_result: i64) -> Self {
        Self {
            status,
            bytes: Vec::new(),
            offset: 0,
            next_offset: None,
            total_size_hint: None,
            truncated: false,
            metric_result,
            metric_bytes: 0,
        }
    }

    fn success(
        status: &'static str,
        bytes: Vec<u8>,
        offset: u64,
        next_offset: Option<u64>,
        total_size_hint: Option<usize>,
        truncated: bool,
    ) -> Self {
        let metric_bytes = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        let metric_result = i64::try_from(bytes.len()).unwrap_or(PAYLOAD_READ_TOO_LARGE);
        Self {
            status,
            bytes,
            offset,
            next_offset,
            total_size_hint: total_size_hint.and_then(|value| u64::try_from(value).ok()),
            truncated,
            metric_result,
            metric_bytes,
        }
    }
}

fn set_component_payload_chunk(results: &mut [Val], outcome: ComponentPayloadReadOutcome) {
    let Some(result) = results.first_mut() else {
        return;
    };
    *result = Val::Record(vec![
        ("status".to_string(), Val::Enum(outcome.status.to_string())),
        (
            "bytes".to_string(),
            Val::List(outcome.bytes.into_iter().map(Val::U8).collect()),
        ),
        ("offset".to_string(), Val::U64(outcome.offset)),
        (
            "next-offset".to_string(),
            component_option_u64(outcome.next_offset),
        ),
        (
            "total-size-hint".to_string(),
            component_option_u64(outcome.total_size_hint),
        ),
        ("truncated".to_string(), Val::Bool(outcome.truncated)),
        ("redaction-applied".to_string(), Val::Bool(false)),
    ]);
}

fn component_option_u64(value: Option<u64>) -> Val {
    Val::Option(value.map(|value| Box::new(Val::U64(value))))
}

fn component_env_read(
    store: wasmtime::StoreContextMut<'_, WasmStoreState>,
    params: &[Val],
    results: &mut [Val],
) {
    let [Val::String(name), Val::U64(max_bytes)] = params else {
        set_component_env_read_error(results, COMPONENT_ENV_READ_INVALID);
        return;
    };
    if name.len() > store.data().host_limits().env_name_max_bytes {
        set_component_env_read_error(results, COMPONENT_ENV_READ_TOO_LARGE);
        return;
    }
    if !store.data().host_grants().can_read_env(name) {
        set_component_env_read_error(results, COMPONENT_ENV_READ_DENIED);
        return;
    }
    let max_bytes = match usize::try_from(*max_bytes) {
        Ok(max_bytes) => max_bytes,
        Err(_) => {
            set_component_env_read_error(results, COMPONENT_ENV_READ_TOO_LARGE);
            return;
        }
    };
    if max_bytes > store.data().host_limits().env_value_max_bytes {
        set_component_env_read_error(results, COMPONENT_ENV_READ_TOO_LARGE);
        return;
    }
    let value = match std::env::var(name) {
        Ok(value) => value,
        Err(_) => {
            set_component_env_read_error(results, COMPONENT_ENV_READ_NOT_FOUND);
            return;
        }
    };
    if value.len() > max_bytes || value.len() > store.data().host_limits().env_value_max_bytes {
        set_component_env_read_error(results, COMPONENT_ENV_READ_TOO_LARGE);
        return;
    }
    set_component_env_read_ok(results, value);
}

fn set_component_env_read_ok(results: &mut [Val], value: String) {
    let Some(result) = results.first_mut() else {
        return;
    };
    *result = Val::Result(Ok(Some(Box::new(Val::String(value)))));
}

fn set_component_env_read_error(results: &mut [Val], message: &str) {
    let Some(result) = results.first_mut() else {
        return;
    };
    *result = Val::Result(Err(Some(Box::new(Val::String(message.to_string())))));
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

    fn consume(
        &self,
        batch: ObservationBatch<'_>,
    ) -> Result<ObservationConsumeReport, PluginRuntimeError> {
        let input = observation_batch_val(&batch);
        let mut state = self.state.lock().map_err(|error| {
            PluginRuntimeError::new(
                "wasm_runtime",
                format!("wasm component state lock poisoned: {error}"),
            )
        })?;
        let fuel_per_call = state.fuel_per_call;
        let consume = state.consume.clone();
        reset_fuel(&mut state.store, fuel_per_call)?;
        let mut results = [Val::Result(Ok(None))];
        state
            .store
            .data_mut()
            .set_payload_snapshot(batch.payload_segments);
        let result = consume.call(&mut state.store, &[input], &mut results);
        state.store.data_mut().clear_payload_snapshot();
        result.map_err(|error| component_call_error(&mut state.store, error))?;
        parse_observation_report(
            &self.instance_id,
            self.queue_capacity,
            batch.trace.trace_id,
            results.into_iter().next().ok_or_else(|| {
                PluginRuntimeError::new("wasm_runtime", "wasm component consume returned no result")
            })?,
        )
    }
}

struct WitComponentObservationState {
    store: WasmStore,
    consume: Func,
    fuel_per_call: u64,
}

fn observation_batch_val(batch: &ObservationBatch<'_>) -> Val {
    Val::Record(vec![
        (
            "trace-id".to_string(),
            Val::String(batch.trace.trace_id.to_string()),
        ),
        (
            "families".to_string(),
            Val::List(observation_families(batch)),
        ),
        (
            "semantic-actions".to_string(),
            Val::List(
                batch
                    .semantic_actions
                    .iter()
                    .map(semantic_action_val)
                    .collect(),
            ),
        ),
        (
            "payload-refs".to_string(),
            Val::List(
                batch
                    .payload_segments
                    .iter()
                    .map(|segment| {
                        Val::Record(vec![
                            (
                                "id".to_string(),
                                Val::String(segment.segment_id.to_string()),
                            ),
                            (
                                "trace-id".to_string(),
                                Val::String(segment.trace_id.to_string()),
                            ),
                        ])
                    })
                    .collect(),
            ),
        ),
    ])
}

fn observation_families(batch: &ObservationBatch<'_>) -> Vec<Val> {
    let mut families = Vec::new();
    if !batch.semantic_actions.is_empty() {
        families.push(Val::Enum("semantic-action".to_string()));
    }
    if !batch.semantic_links.is_empty() {
        families.push(Val::Enum("semantic-action-link".to_string()));
    }
    if !batch.payload_segments.is_empty() {
        families.push(Val::Enum("payload-metadata".to_string()));
    }
    families
}

fn semantic_action_val(action: &SemanticAction) -> Val {
    Val::Record(vec![
        (
            "trace-id".to_string(),
            Val::String(action.trace_id.to_string()),
        ),
        (
            "action-id".to_string(),
            Val::String(action.action_id.clone()),
        ),
        (
            "kind".to_string(),
            Val::String(format!("{:?}", action.kind)),
        ),
        ("summary".to_string(), Val::String(action.title.clone())),
    ])
}

fn parse_observation_report(
    instance_id: &str,
    queue_capacity: u32,
    trace_id: TraceId,
    value: Val,
) -> Result<ObservationConsumeReport, PluginRuntimeError> {
    let report = match value {
        Val::Result(Ok(Some(ok))) => *ok,
        Val::Result(Ok(None)) => {
            return Err(PluginRuntimeError::new(
                "wasm_runtime",
                "wasm component consume returned ok without observation-report",
            ));
        }
        Val::Result(Err(Some(error))) => {
            let message = match *error {
                Val::String(message) => message,
                other => format!("{other:?}"),
            };
            return Err(PluginRuntimeError::new(
                "wasm_runtime",
                format!("wasm component consume returned error: {message}"),
            ));
        }
        Val::Result(Err(None)) => {
            return Err(PluginRuntimeError::new(
                "wasm_runtime",
                "wasm component consume returned error without message",
            ));
        }
        other => {
            return Err(PluginRuntimeError::new(
                "wasm_runtime",
                format!("wasm component consume returned invalid result {other:?}"),
            ));
        }
    };
    let fields = match report {
        Val::Record(fields) => fields,
        other => {
            return Err(PluginRuntimeError::new(
                "wasm_runtime",
                format!("wasm component consume returned invalid report {other:?}"),
            ));
        }
    };
    let _observed_records = report_field_u64(&fields, "observed-records")?;
    let dropped_records = report_field_u64(&fields, "dropped-records")?;
    if dropped_records == 0 {
        return Ok(ObservationConsumeReport::empty());
    }
    Ok(ObservationConsumeReport {
        dropped_records: vec![PluginDroppedRecord {
            trace_id,
            plugin_instance: instance_id.to_string(),
            reason: "wasm_component_reported_drop".to_string(),
            queue_capacity: Some(queue_capacity),
            dropped_records,
        }],
    })
}

fn report_field_u64(fields: &[(String, Val)], name: &str) -> Result<u64, PluginRuntimeError> {
    match fields.iter().find(|(field, _)| field == name) {
        Some((_, Val::U64(value))) => Ok(*value),
        Some((_, other)) => Err(PluginRuntimeError::new(
            "wasm_runtime",
            format!("wasm component report field {name} has invalid value {other:?}"),
        )),
        None => Err(PluginRuntimeError::new(
            "wasm_runtime",
            format!("wasm component report missing field {name}"),
        )),
    }
}

fn component_call_error(
    store: &mut WasmStore,
    error: impl std::fmt::Display,
) -> PluginRuntimeError {
    if store.get_fuel().map(|fuel| fuel == 0).unwrap_or(false) {
        PluginRuntimeError::new(
            "wasm_runtime",
            format!("wasm fuel exhausted during wasm component observation consume: {error}"),
        )
    } else {
        PluginRuntimeError::new(
            "wasm_runtime",
            format!("wasm component observation consume trapped: {error}"),
        )
    }
}
