use std::time::Instant;

use plugin_system::PluginRuntimeError;
use wasmtime::Engine;
use wasmtime::component::{Linker as ComponentLinker, Val};

use crate::component_observation::alert_host::register_alert_interface;
use crate::component_observation::post_trace_host::register_post_trace_interfaces;
use crate::engine::WasmStoreState;
use crate::host::component_read_config;

const HOST_IMPORT: &str = "actrail:plugin/host@0.2.0";
const OBSERVATION_CONTEXT_IMPORT: &str = "actrail:plugin/observation-context-read@0.2.0";
const READ_CONFIG_IMPORT: &str = "read-config";
const ENV_READ_IMPORT: &str = "env-read";
const READ_PAYLOAD_IMPORT: &str = "read-payload";
const TRACE_CONTEXT_GET_IMPORT: &str = "trace-context-get";
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

pub(super) fn component_linker(
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
    let mut observation_context = linker
        .instance(OBSERVATION_CONTEXT_IMPORT)
        .map_err(|error| {
            PluginRuntimeError::new(
                "wasm_runtime",
                format!("define wasm observation context instance failed: {error}"),
            )
        })?;
    observation_context
        .func_new(TRACE_CONTEXT_GET_IMPORT, |store, _ty, _params, results| {
            set_trace_context_result(store.data(), results);
            Ok(())
        })
        .map_err(|error| {
            PluginRuntimeError::new(
                "wasm_runtime",
                format!("define trace-context-get import failed: {error}"),
            )
        })?;
    register_post_trace_interfaces(&mut linker)?;
    register_alert_interface(&mut linker)?;
    Ok(linker)
}

fn set_trace_context_result(state: &WasmStoreState, results: &mut [Val]) {
    let Some(result) = results.first_mut() else {
        return;
    };
    let Some(context) = state.observation_trace_context() else {
        *result = Val::Result(Err(Some(Box::new(Val::String(
            "observation trace context is unavailable".to_string(),
        )))));
        return;
    };
    *result = Val::Result(Ok(Some(Box::new(Val::Record(vec![
        (
            "working-directory".to_string(),
            Val::Option(
                context
                    .working_directory
                    .clone()
                    .map(Val::String)
                    .map(Box::new),
            ),
        ),
        (
            "alert-token".to_string(),
            Val::Option(context.alert_token.as_ref().map(|token| {
                Box::new(Val::List(
                    token.as_bytes().iter().copied().map(Val::U8).collect(),
                ))
            })),
        ),
    ])))));
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
    let next_offset = truncated
        .then(|| offset.checked_add(u64::try_from(count).unwrap_or(u64::MAX)))
        .flatten();
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
        (field == name)
            .then_some(value)
            .and_then(|value| match value {
                Val::String(value) => Some(value.as_str()),
                _ => None,
            })
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
    if let Some(result) = results.first_mut() {
        *result = Val::Result(Ok(Some(Box::new(Val::String(value)))));
    }
}

fn set_component_env_read_error(results: &mut [Val], message: &str) {
    if let Some(result) = results.first_mut() {
        *result = Val::Result(Err(Some(Box::new(Val::String(message.to_string())))));
    }
}
