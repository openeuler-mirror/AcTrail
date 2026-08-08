//! Legacy core-module configuration, environment, payload, and current-context hostcalls.

use std::time::Instant;

use plugin_system::{CONTROL_DECISION_SUMMARY_QUERY, FILE_POLICY_MATCHED_RULE_QUERY};
use wasmtime::Caller;
use wasmtime::component::Val;

use crate::engine::WasmStoreState;

use super::{decision_summary_response, exported_memory, guest_range, matched_rule_response};

const ENV_READ_DENIED: i64 = -1;
const ENV_READ_NOT_FOUND: i64 = -2;
const ENV_READ_INVALID: i64 = -3;
const ENV_READ_TOO_LARGE: i64 = -4;
const PAYLOAD_READ_DENIED: i64 = -1;
const PAYLOAD_READ_NOT_FOUND: i64 = -2;
const PAYLOAD_READ_INVALID: i64 = -3;
const PAYLOAD_READ_TOO_LARGE: i64 = -4;
const CONTEXT_QUERY_DENIED: i64 = -1;
const CONTEXT_QUERY_NOT_FOUND: i64 = -2;
const CONTEXT_QUERY_INVALID: i64 = -3;
const CONTEXT_QUERY_TOO_LARGE: i64 = -4;
const FILE_POLICY_READ_DENIED: i64 = -1;
const FILE_POLICY_READ_NOT_FOUND: i64 = -2;
const FILE_POLICY_READ_INVALID: i64 = -3;
const FILE_POLICY_READ_TOO_LARGE: i64 = -4;

mod component_config {
    pub mod status {
        pub const OK: &str = "ok";
        pub const NOT_CONFIGURED: &str = "not-configured";
        pub const TOO_LARGE: &str = "too-large";
    }

    pub mod field {
        pub const STATUS: &str = "status";
        pub const BYTES: &str = "bytes";
        pub const OFFSET: &str = "offset";
        pub const NEXT_OFFSET: &str = "next-offset";
        pub const TOTAL_SIZE_HINT: &str = "total-size-hint";
        pub const TRUNCATED: &str = "truncated";
    }
}

pub(crate) fn component_read_config(
    store: wasmtime::StoreContextMut<'_, WasmStoreState>,
    params: &[Val],
    results: &mut [Val],
) {
    let [Val::U64(offset), Val::U64(max_bytes)] = params else {
        set_component_config_chunk(
            results,
            ComponentConfigReadOutcome::empty(component_config::status::TOO_LARGE),
        );
        return;
    };
    let Ok(max_bytes) = usize::try_from(*max_bytes) else {
        set_component_config_chunk(
            results,
            ComponentConfigReadOutcome::empty(component_config::status::TOO_LARGE),
        );
        return;
    };
    if max_bytes > store.data().host_limits().plugin_config_read_max_bytes {
        set_component_config_chunk(
            results,
            ComponentConfigReadOutcome::empty(component_config::status::TOO_LARGE),
        );
        return;
    }
    let Some(config) = store.data().plugin_config() else {
        set_component_config_chunk(
            results,
            ComponentConfigReadOutcome::empty(component_config::status::NOT_CONFIGURED),
        );
        return;
    };
    let offset_usize = usize::try_from(*offset).unwrap_or(usize::MAX);
    if offset_usize >= config.len() {
        set_component_config_chunk(
            results,
            ComponentConfigReadOutcome::success(Vec::new(), *offset, None, config.len(), false),
        );
        return;
    }
    let available = &config[offset_usize..];
    let count = available.len().min(max_bytes);
    let truncated = available.len() > count;
    let bytes = available[..count].to_vec();
    let next_offset = if truncated {
        offset.checked_add(u64::try_from(count).unwrap_or(u64::MAX))
    } else {
        None
    };
    set_component_config_chunk(
        results,
        ComponentConfigReadOutcome::success(bytes, *offset, next_offset, config.len(), truncated),
    );
}

#[derive(Debug)]
struct ComponentConfigReadOutcome {
    status: &'static str,
    bytes: Vec<u8>,
    offset: u64,
    next_offset: Option<u64>,
    total_size_hint: Option<u64>,
    truncated: bool,
}

impl ComponentConfigReadOutcome {
    fn empty(status: &'static str) -> Self {
        Self {
            status,
            bytes: Vec::new(),
            offset: 0,
            next_offset: None,
            total_size_hint: None,
            truncated: false,
        }
    }

    fn success(
        bytes: Vec<u8>,
        offset: u64,
        next_offset: Option<u64>,
        total_size_hint: usize,
        truncated: bool,
    ) -> Self {
        Self {
            status: component_config::status::OK,
            bytes,
            offset,
            next_offset,
            total_size_hint: u64::try_from(total_size_hint).ok(),
            truncated,
        }
    }
}

fn set_component_config_chunk(results: &mut [Val], outcome: ComponentConfigReadOutcome) {
    let Some(result) = results.first_mut() else {
        return;
    };
    *result = Val::Record(vec![
        (
            component_config::field::STATUS.to_string(),
            Val::Enum(outcome.status.to_string()),
        ),
        (
            component_config::field::BYTES.to_string(),
            Val::List(outcome.bytes.into_iter().map(Val::U8).collect()),
        ),
        (
            component_config::field::OFFSET.to_string(),
            Val::U64(outcome.offset),
        ),
        (
            component_config::field::NEXT_OFFSET.to_string(),
            component_option_u64(outcome.next_offset),
        ),
        (
            component_config::field::TOTAL_SIZE_HINT.to_string(),
            component_option_u64(outcome.total_size_hint),
        ),
        (
            component_config::field::TRUNCATED.to_string(),
            Val::Bool(outcome.truncated),
        ),
    ]);
}

fn component_option_u64(value: Option<u64>) -> Val {
    Val::Option(value.map(|value| Box::new(Val::U64(value))))
}

pub(super) fn env_read(
    caller: &mut Caller<'_, WasmStoreState>,
    name_ptr: i32,
    name_len: i32,
    out_ptr: i32,
    max_len: i32,
) -> i64 {
    let Ok(memory) = exported_memory(caller) else {
        return ENV_READ_INVALID;
    };
    let Ok((name_offset, name_len)) = guest_range(name_ptr, name_len) else {
        return ENV_READ_INVALID;
    };
    if name_len > caller.data().host_limits().env_name_max_bytes {
        return ENV_READ_TOO_LARGE;
    }
    let mut name_bytes = vec![0_u8; name_len];
    if memory
        .read(&mut *caller, name_offset, &mut name_bytes)
        .is_err()
    {
        return ENV_READ_INVALID;
    }
    let Ok(name) = std::str::from_utf8(&name_bytes) else {
        return ENV_READ_INVALID;
    };
    if !caller.data().host_grants().can_read_env(name) {
        return ENV_READ_DENIED;
    }
    let value = match std::env::var(name) {
        Ok(value) => value,
        Err(_) => return ENV_READ_NOT_FOUND,
    };
    let value_bytes = value.as_bytes();
    let Ok((out_offset, max_len)) = guest_range(out_ptr, max_len) else {
        return ENV_READ_INVALID;
    };
    if max_len > caller.data().host_limits().env_value_max_bytes {
        return ENV_READ_TOO_LARGE;
    }
    if value_bytes.len() > max_len {
        return ENV_READ_TOO_LARGE;
    }
    if memory.write(&mut *caller, out_offset, value_bytes).is_err() {
        return ENV_READ_INVALID;
    }
    i64::try_from(value_bytes.len()).unwrap_or(ENV_READ_TOO_LARGE)
}

pub(super) fn payload_read(
    caller: &mut Caller<'_, WasmStoreState>,
    ref_ptr: i32,
    ref_len: i32,
    offset: i64,
    out_ptr: i32,
    max_len: i32,
) -> i64 {
    let started_at = Instant::now();
    let outcome = payload_read_inner(caller, ref_ptr, ref_len, offset, out_ptr, max_len);
    caller.data().hostcall_metrics().record_payload_read(
        outcome.result,
        outcome.bytes,
        outcome.truncated,
        started_at.elapsed(),
    );
    outcome.result
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PayloadReadOutcome {
    result: i64,
    bytes: u64,
    truncated: bool,
}

impl PayloadReadOutcome {
    fn result(result: i64) -> Self {
        Self {
            result,
            bytes: 0,
            truncated: false,
        }
    }

    fn success(bytes: usize, truncated: bool) -> Self {
        let Ok(result) = i64::try_from(bytes) else {
            return Self::result(PAYLOAD_READ_TOO_LARGE);
        };
        Self {
            result,
            bytes: u64::try_from(bytes).unwrap_or(u64::MAX),
            truncated,
        }
    }
}

fn payload_read_inner(
    caller: &mut Caller<'_, WasmStoreState>,
    ref_ptr: i32,
    ref_len: i32,
    offset: i64,
    out_ptr: i32,
    max_len: i32,
) -> PayloadReadOutcome {
    if !caller.data().host_grants().can_read_payload() {
        return PayloadReadOutcome::result(PAYLOAD_READ_DENIED);
    }
    if offset < 0 {
        return PayloadReadOutcome::result(PAYLOAD_READ_INVALID);
    }
    let Ok(memory) = exported_memory(caller) else {
        return PayloadReadOutcome::result(PAYLOAD_READ_INVALID);
    };
    let Ok((ref_offset, ref_len)) = guest_range(ref_ptr, ref_len) else {
        return PayloadReadOutcome::result(PAYLOAD_READ_INVALID);
    };
    if ref_len > caller.data().host_limits().payload_ref_max_bytes {
        return PayloadReadOutcome::result(PAYLOAD_READ_TOO_LARGE);
    }
    let mut ref_bytes = vec![0_u8; ref_len];
    if memory
        .read(&mut *caller, ref_offset, &mut ref_bytes)
        .is_err()
    {
        return PayloadReadOutcome::result(PAYLOAD_READ_INVALID);
    }
    let Ok(ref_id) = std::str::from_utf8(&ref_bytes) else {
        return PayloadReadOutcome::result(PAYLOAD_READ_INVALID);
    };
    let Ok((out_offset, max_len)) = guest_range(out_ptr, max_len) else {
        return PayloadReadOutcome::result(PAYLOAD_READ_INVALID);
    };
    if max_len > caller.data().host_limits().payload_read_max_bytes {
        return PayloadReadOutcome::result(PAYLOAD_READ_TOO_LARGE);
    }
    let Some(payload) = caller.data().payload_entry(ref_id) else {
        return PayloadReadOutcome::result(PAYLOAD_READ_NOT_FOUND);
    };
    if !caller
        .data()
        .host_grants()
        .can_read_payload_source(payload.source_boundary)
    {
        return PayloadReadOutcome::result(PAYLOAD_READ_DENIED);
    }
    let Some(bytes) = payload.bytes.as_deref() else {
        return PayloadReadOutcome::result(PAYLOAD_READ_NOT_FOUND);
    };
    let offset = usize::try_from(offset).unwrap_or(usize::MAX);
    if offset >= bytes.len() {
        return PayloadReadOutcome::success(0, false);
    }
    let available = &bytes[offset..];
    let count = available.len().min(max_len);
    let truncated = available.len() > count;
    let chunk = available[..count].to_vec();
    if memory.write(&mut *caller, out_offset, &chunk).is_err() {
        return PayloadReadOutcome::result(PAYLOAD_READ_INVALID);
    }
    PayloadReadOutcome::success(count, truncated)
}

pub(super) fn context_query(
    caller: &mut Caller<'_, WasmStoreState>,
    context_ptr: i32,
    context_len: i32,
    query_ptr: i32,
    query_len: i32,
    out_ptr: i32,
    max_len: i32,
) -> i64 {
    if !caller.data().host_grants().can_query_context() {
        return CONTEXT_QUERY_DENIED;
    }
    let Ok(memory) = exported_memory(caller) else {
        return CONTEXT_QUERY_INVALID;
    };
    let Ok((context_offset, context_len)) = guest_range(context_ptr, context_len) else {
        return CONTEXT_QUERY_INVALID;
    };
    if context_len > caller.data().host_limits().context_ref_max_bytes {
        return CONTEXT_QUERY_TOO_LARGE;
    }
    let Ok((query_offset, query_len)) = guest_range(query_ptr, query_len) else {
        return CONTEXT_QUERY_INVALID;
    };
    if query_len > caller.data().host_limits().context_query_max_bytes {
        return CONTEXT_QUERY_TOO_LARGE;
    }
    let Ok((out_offset, max_len)) = guest_range(out_ptr, max_len) else {
        return CONTEXT_QUERY_INVALID;
    };
    if max_len > caller.data().host_limits().context_read_max_bytes {
        return CONTEXT_QUERY_TOO_LARGE;
    }
    let mut context_bytes = vec![0_u8; context_len];
    if memory
        .read(&mut *caller, context_offset, &mut context_bytes)
        .is_err()
    {
        return CONTEXT_QUERY_INVALID;
    }
    let Ok(context_ref) = std::str::from_utf8(&context_bytes) else {
        return CONTEXT_QUERY_INVALID;
    };
    let mut query_bytes = vec![0_u8; query_len];
    if memory
        .read(&mut *caller, query_offset, &mut query_bytes)
        .is_err()
    {
        return CONTEXT_QUERY_INVALID;
    }
    let Ok(query) = std::str::from_utf8(&query_bytes) else {
        return CONTEXT_QUERY_INVALID;
    };
    let Some(context) = caller.data().control_context().cloned() else {
        return CONTEXT_QUERY_NOT_FOUND;
    };
    if context.context_ref != context_ref || query != CONTROL_DECISION_SUMMARY_QUERY {
        return CONTEXT_QUERY_NOT_FOUND;
    }
    let response = decision_summary_response(&context);
    let response_bytes = response.as_bytes();
    if response_bytes.len() > max_len {
        return CONTEXT_QUERY_TOO_LARGE;
    }
    if memory
        .write(&mut *caller, out_offset, response_bytes)
        .is_err()
    {
        return CONTEXT_QUERY_INVALID;
    }
    i64::try_from(response_bytes.len()).unwrap_or(CONTEXT_QUERY_TOO_LARGE)
}

pub(super) fn file_access_current_match_get(
    caller: &mut Caller<'_, WasmStoreState>,
    context_ptr: i32,
    context_len: i32,
    query_ptr: i32,
    query_len: i32,
    out_ptr: i32,
    max_len: i32,
) -> i64 {
    if !caller
        .data()
        .host_grants()
        .can_get_current_file_access_match()
    {
        return FILE_POLICY_READ_DENIED;
    }
    let Ok(memory) = exported_memory(caller) else {
        return FILE_POLICY_READ_INVALID;
    };
    let Ok((context_offset, context_len)) = guest_range(context_ptr, context_len) else {
        return FILE_POLICY_READ_INVALID;
    };
    if context_len
        > caller
            .data()
            .host_limits()
            .file_policy_context_ref_max_bytes
    {
        return FILE_POLICY_READ_TOO_LARGE;
    }
    let Ok((query_offset, query_len)) = guest_range(query_ptr, query_len) else {
        return FILE_POLICY_READ_INVALID;
    };
    if query_len > caller.data().host_limits().file_policy_query_max_bytes {
        return FILE_POLICY_READ_TOO_LARGE;
    }
    let Ok((out_offset, max_len)) = guest_range(out_ptr, max_len) else {
        return FILE_POLICY_READ_INVALID;
    };
    if max_len > caller.data().host_limits().file_policy_io_max_bytes {
        return FILE_POLICY_READ_TOO_LARGE;
    }
    let mut context_bytes = vec![0_u8; context_len];
    if memory
        .read(&mut *caller, context_offset, &mut context_bytes)
        .is_err()
    {
        return FILE_POLICY_READ_INVALID;
    }
    let Ok(context_ref) = std::str::from_utf8(&context_bytes) else {
        return FILE_POLICY_READ_INVALID;
    };
    let mut query_bytes = vec![0_u8; query_len];
    if memory
        .read(&mut *caller, query_offset, &mut query_bytes)
        .is_err()
    {
        return FILE_POLICY_READ_INVALID;
    }
    let Ok(query) = std::str::from_utf8(&query_bytes) else {
        return FILE_POLICY_READ_INVALID;
    };
    let Some(context) = caller.data().file_policy_context() else {
        return FILE_POLICY_READ_NOT_FOUND;
    };
    if context.context_ref != context_ref || query != FILE_POLICY_MATCHED_RULE_QUERY {
        return FILE_POLICY_READ_NOT_FOUND;
    }
    let response = matched_rule_response(context);
    let response_bytes = response.as_bytes();
    if response_bytes.len() > max_len {
        return FILE_POLICY_READ_TOO_LARGE;
    }
    if memory
        .write(&mut *caller, out_offset, response_bytes)
        .is_err()
    {
        return FILE_POLICY_READ_INVALID;
    }
    i64::try_from(response_bytes.len()).unwrap_or(FILE_POLICY_READ_TOO_LARGE)
}
