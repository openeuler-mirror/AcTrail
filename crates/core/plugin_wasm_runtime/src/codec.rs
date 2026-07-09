use std::sync::{Arc, Mutex};

use plugin_system::{
    LlmCodecDecoded, LlmCodecOutcome, LlmCodecPlugin, LlmCodecRequest, LlmCodecSseEvent,
    PluginHostGrants, PluginHostcallMetricsSource, PluginManifest, PluginRuntimeError,
    PluginRuntimeKind, PluginWasmAbi,
};
use serde_json::{Value, json};
use wasmtime::{Memory, Module, TypedFunc};

use crate::engine::{
    WasmHostcallMetrics, WasmStore, call_error, fuel_per_call, host_limits, limited_store,
    memory_max_bytes, metered_engine, reset_fuel,
};
use crate::host::host_linker;
use crate::memory::{read_guest_bytes, write_guest_bytes};

const CODEC_OUTPUT_MAX_BYTES: usize = 8 * 1024 * 1024;

pub fn build_wasm_llm_codec_plugin(
    instance_id: impl Into<String>,
    manifest: &PluginManifest,
    plugin_config: Option<&str>,
    host_grants: PluginHostGrants,
) -> Result<WasmLlmCodecPlugin, PluginRuntimeError> {
    let wasm = manifest.selected_wasm().ok_or_else(|| {
        PluginRuntimeError::new(
            "wasm_runtime",
            "wasm plugin manifest missing [runtime.wasm]",
        )
    })?;
    match wasm.abi {
        PluginWasmAbi::LegacyModule => WasmLlmCodecPlugin::load(
            instance_id,
            manifest,
            plugin_config.unwrap_or_default(),
            host_grants,
        ),
        PluginWasmAbi::WitComponent => Err(PluginRuntimeError::new(
            "wasm_runtime",
            "llm-codec plugins currently support legacy-module ABI only",
        )),
    }
}

pub struct WasmLlmCodecPlugin {
    instance_id: String,
    plugin_id: String,
    host_grants: Vec<String>,
    hostcall_metrics: Arc<WasmHostcallMetrics>,
    state: Mutex<WasmLlmCodecState>,
}

impl WasmLlmCodecPlugin {
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
        let fuel_per_call = fuel_per_call(manifest);
        let memory_max_bytes = memory_max_bytes(manifest)?;
        let host_limits = host_limits(manifest)?;
        let host_grant_values = host_grants.to_wire_values();
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
        let decode_envelope = instance
            .get_typed_func::<(i32, i32), i64>(&mut store, "actrail_llm_codec_decode")
            .ok();
        let decode_request = instance
            .get_typed_func::<(i32, i32), i64>(&mut store, "actrail_llm_codec_decode_request")
            .ok();
        let decode_sse_event = instance
            .get_typed_func::<(i32, i32), i64>(&mut store, "actrail_llm_codec_decode_sse_event")
            .ok();
        if decode_envelope.is_none() && decode_request.is_none() && decode_sse_event.is_none() {
            return Err(PluginRuntimeError::new(
                "wasm_runtime",
                "wasm llm-codec plugin missing decode export",
            ));
        }
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
            hostcall_metrics,
            state: Mutex::new(WasmLlmCodecState {
                store,
                memory,
                alloc,
                decode_envelope,
                decode_request,
                decode_sse_event,
                fuel_per_call,
            }),
        })
    }

    fn decode_raw(
        &self,
        input: &[u8],
        decode: TypedFunc<(i32, i32), i64>,
        call_label: &'static str,
    ) -> Result<LlmCodecOutcome, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|error| format!("wasm llm-codec state lock poisoned: {error}"))?;
        let memory = state.memory;
        let alloc = state.alloc.clone();
        let fuel_per_call = state.fuel_per_call;
        reset_fuel(&mut state.store, fuel_per_call).map_err(|error| error.message)?;
        let (ptr, len) = write_guest_bytes(&mut state.store, memory, alloc, input)
            .map_err(|error| error.message)?;
        let packed = decode
            .call(&mut state.store, (ptr, len))
            .map_err(|error| call_error(&mut state.store, call_label, error).message)?;
        let (out_ptr, out_len) = unpack_guest_slice(packed)?;
        let output = read_guest_bytes(
            &mut state.store,
            memory,
            out_ptr,
            out_len,
            CODEC_OUTPUT_MAX_BYTES,
        )
        .map_err(|error| error.message)?;
        parse_codec_output(&output)
    }

    fn decode_envelope(&self, envelope: Value) -> Result<LlmCodecOutcome, String> {
        let input = serde_json::to_vec(&envelope)
            .map_err(|error| format!("encode llm-codec envelope failed: {error}"))?;
        let decode = self
            .state
            .lock()
            .map_err(|error| format!("wasm llm-codec state lock poisoned: {error}"))?
            .decode_envelope
            .clone()
            .ok_or_else(|| "wasm llm-codec plugin has no envelope decode export".to_string())?;
        self.decode_raw(&input, decode, "wasm llm-codec decode")
    }
}

impl LlmCodecPlugin for WasmLlmCodecPlugin {
    fn instance_id(&self) -> &str {
        &self.instance_id
    }

    fn plugin_id(&self) -> &str {
        &self.plugin_id
    }

    fn decode_request(&self, request: LlmCodecRequest<'_>) -> Result<LlmCodecOutcome, String> {
        let direct = self
            .state
            .lock()
            .map_err(|error| format!("wasm llm-codec state lock poisoned: {error}"))?
            .decode_request
            .clone();
        if let Some(decode) = direct {
            return self.decode_raw(request.body, decode, "wasm llm-codec decode request");
        }
        self.decode_envelope(json!({
            "schema_version": "actrail.llm-codec.v0",
            "phase": "request",
            "method": request.method,
            "authority": request.authority,
            "path": request.path,
            "body": request.body,
        }))
    }

    fn decode_sse_event(&self, event: LlmCodecSseEvent<'_>) -> Result<LlmCodecOutcome, String> {
        let direct = self
            .state
            .lock()
            .map_err(|error| format!("wasm llm-codec state lock poisoned: {error}"))?
            .decode_sse_event
            .clone();
        if let Some(decode) = direct {
            return self.decode_raw(
                event.data.as_bytes(),
                decode,
                "wasm llm-codec decode sse event",
            );
        }
        self.decode_envelope(json!({
            "schema_version": "actrail.llm-codec.v0",
            "phase": "sse-event",
            "index": event.index,
            "event_type": event.event_type,
            "id": event.id,
            "data": event.data,
        }))
    }
}

impl WasmLlmCodecPlugin {
    pub fn runtime_kind(&self) -> PluginRuntimeKind {
        PluginRuntimeKind::Wasm
    }

    pub fn host_grants(&self) -> Vec<String> {
        self.host_grants.clone()
    }

    pub fn hostcall_metrics_source(&self) -> Option<Arc<dyn PluginHostcallMetricsSource>> {
        Some(self.hostcall_metrics.clone())
    }
}

struct WasmLlmCodecState {
    store: WasmStore,
    memory: Memory,
    alloc: TypedFunc<i32, i32>,
    decode_envelope: Option<TypedFunc<(i32, i32), i64>>,
    decode_request: Option<TypedFunc<(i32, i32), i64>>,
    decode_sse_event: Option<TypedFunc<(i32, i32), i64>>,
    fuel_per_call: u64,
}

fn unpack_guest_slice(packed: i64) -> Result<(i32, i32), String> {
    if packed < 0 {
        return Err(format!("wasm llm-codec returned negative result {packed}"));
    }
    let raw = u64::try_from(packed).map_err(|error| error.to_string())?;
    let ptr = i32::try_from(raw >> 32).map_err(|error| format!("ptr overflow: {error}"))?;
    let len = i32::try_from(raw & 0xffff_ffff).map_err(|error| format!("len overflow: {error}"))?;
    Ok((ptr, len))
}

fn parse_codec_output(bytes: &[u8]) -> Result<LlmCodecOutcome, String> {
    let value = serde_json::from_slice::<Value>(bytes)
        .map_err(|error| format!("parse llm-codec output failed: {error}"))?;
    let status = value
        .get("status")
        .and_then(Value::as_str)
        .ok_or_else(|| "llm-codec output missing status".to_string())?;
    match status {
        "no_match" => Ok(LlmCodecOutcome::NoMatch),
        "decoded" => Ok(LlmCodecOutcome::Decoded(LlmCodecDecoded {
            classifier_id: optional_string(&value, "classifier_id"),
            protocol_id: optional_string(&value, "protocol_id"),
            provider_id: optional_string(&value, "provider_id"),
            model: optional_string(&value, "model"),
            body: byte_array(&value, "body")?,
        })),
        other => Err(format!("unsupported llm-codec status {other}")),
    }
}

fn optional_string(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn byte_array(value: &Value, key: &str) -> Result<Vec<u8>, String> {
    let items = value
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("llm-codec decoded output missing {key} byte array"))?;
    let mut bytes = Vec::with_capacity(items.len());
    for item in items {
        let byte = item
            .as_u64()
            .ok_or_else(|| format!("llm-codec {key} contains non-integer byte"))?;
        bytes.push(u8::try_from(byte).map_err(|error| format!("byte overflow: {error}"))?);
    }
    Ok(bytes)
}
