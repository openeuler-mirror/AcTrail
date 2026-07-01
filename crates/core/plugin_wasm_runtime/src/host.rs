use std::time::Instant;

use plugin_system::{
    CONTROL_DECISION_SUMMARY_QUERY, FILE_POLICY_MATCHED_RULE_QUERY, FilePolicyApplyMode,
    FilePolicyApplyPrecondition, FilePolicyApplyRequest, FilePolicyDecision, FilePolicyListFilter,
    FilePolicyMatchDryRunRequest, FilePolicyOperation, FilePolicyPatchItem, FilePolicyPatchOp,
    FilePolicyReadContext, FilePolicyRuleDraft, PluginRuntimeError,
};
use wasmtime::component::Val;
use wasmtime::{Caller, Engine, Linker, Memory};

use crate::engine::{ControlContextSnapshot, WasmStoreState};

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
const FILE_POLICY_RULES_DENIED: i64 = -1;
const FILE_POLICY_RULES_NOT_FOUND: i64 = -2;
const FILE_POLICY_RULES_INVALID: i64 = -3;
const FILE_POLICY_RULES_TOO_LARGE: i64 = -4;
const FILE_POLICY_RULES_REJECTED: i64 = -5;
const FILE_POLICY_RULES_BINARY_VERSION: u8 = 1;
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

mod legacy_policy_text {
    pub const CONTEXT_QUERY_SCHEMA_VERSION: &str = "context-query.v1";
    pub const CURRENT_MATCH_SCHEMA_VERSION: &str = "file-access.current-match-get.v1";

    pub mod field {
        pub const VERSION: &str = "version";
        pub const SUBJECT: &str = "subject";
        pub const RULE_ID: &str = "rule_id";
        pub const DECISION: &str = "decision";
        pub const FALLBACK: &str = "fallback";
        pub const TIMEOUT_MS: &str = "timeout_ms";
        pub const CONCURRENCY_LIMIT: &str = "concurrency_limit";
        pub const OPERATION: &str = "operation";
        pub const PLUGIN_INSTANCE: &str = "plugin_instance";
        pub const PATH: &str = "path";
        pub const TARGET_SUMMARY: &str = "target_summary";
        pub const DECISION_ID: &str = "decision_id";
        pub const TRACE_ID: &str = "trace_id";
        pub const ACTOR_PROCESS_IDENTITY: &str = "actor_process_identity";
    }
}

pub(crate) fn host_linker(engine: &Engine) -> Result<Linker<WasmStoreState>, PluginRuntimeError> {
    let mut linker = Linker::new(engine);
    linker
        .func_wrap(
            "actrail_host",
            "env_read",
            |mut caller: Caller<'_, WasmStoreState>,
             name_ptr: i32,
             name_len: i32,
             out_ptr: i32,
             max_len: i32|
             -> i64 { env_read(&mut caller, name_ptr, name_len, out_ptr, max_len) },
        )
        .map_err(|error| {
            PluginRuntimeError::new(
                "wasm_runtime",
                format!("define wasm env_read hostcall failed: {error}"),
            )
        })?;
    linker
        .func_wrap(
            "actrail_host",
            "payload_read",
            |mut caller: Caller<'_, WasmStoreState>,
             ref_ptr: i32,
             ref_len: i32,
             offset: i64,
             out_ptr: i32,
             max_len: i32|
             -> i64 {
                payload_read(&mut caller, ref_ptr, ref_len, offset, out_ptr, max_len)
            },
        )
        .map_err(|error| {
            PluginRuntimeError::new(
                "wasm_runtime",
                format!("define wasm payload_read hostcall failed: {error}"),
            )
        })?;
    linker
        .func_wrap(
            "actrail_host",
            "context_query",
            |mut caller: Caller<'_, WasmStoreState>,
             context_ptr: i32,
             context_len: i32,
             query_ptr: i32,
             query_len: i32,
             out_ptr: i32,
             max_len: i32|
             -> i64 {
                context_query(
                    &mut caller,
                    context_ptr,
                    context_len,
                    query_ptr,
                    query_len,
                    out_ptr,
                    max_len,
                )
            },
        )
        .map_err(|error| {
            PluginRuntimeError::new(
                "wasm_runtime",
                format!("define wasm context_query hostcall failed: {error}"),
            )
        })?;
    linker
        .func_wrap(
            "actrail_host",
            "file_access_current_match_get",
            |mut caller: Caller<'_, WasmStoreState>,
             context_ptr: i32,
             context_len: i32,
             query_ptr: i32,
             query_len: i32,
             out_ptr: i32,
             max_len: i32|
             -> i64 {
                file_access_current_match_get(
                    &mut caller,
                    context_ptr,
                    context_len,
                    query_ptr,
                    query_len,
                    out_ptr,
                    max_len,
                )
            },
        )
        .map_err(|error| {
            PluginRuntimeError::new(
                "wasm_runtime",
                format!("define wasm file_access_current_match_get hostcall failed: {error}"),
            )
        })?;
    linker
        .func_wrap(
            "actrail_host",
            "file_policy_rules_version_get",
            |caller: Caller<'_, WasmStoreState>| -> i64 { file_policy_rules_version_get(caller) },
        )
        .map_err(|error| {
            PluginRuntimeError::new(
                "wasm_runtime",
                format!("define wasm file_policy_rules_version_get hostcall failed: {error}"),
            )
        })?;
    linker
        .func_wrap(
            "actrail_host",
            "file_policy_rules_list",
            |mut caller: Caller<'_, WasmStoreState>,
             filter_ptr: i32,
             filter_len: i32,
             cursor_ptr: i32,
             cursor_len: i32,
             limit: i32,
             out_ptr: i32,
             max_len: i32|
             -> i64 {
                file_policy_rules_list(
                    &mut caller,
                    filter_ptr,
                    filter_len,
                    cursor_ptr,
                    cursor_len,
                    limit,
                    out_ptr,
                    max_len,
                )
            },
        )
        .map_err(|error| {
            PluginRuntimeError::new(
                "wasm_runtime",
                format!("define wasm file_policy_rules_list hostcall failed: {error}"),
            )
        })?;
    linker
        .func_wrap(
            "actrail_host",
            "file_policy_rules_match_dry_run",
            |mut caller: Caller<'_, WasmStoreState>,
             request_ptr: i32,
             request_len: i32,
             out_ptr: i32,
             max_len: i32|
             -> i64 {
                file_policy_rules_match_dry_run(
                    &mut caller,
                    request_ptr,
                    request_len,
                    out_ptr,
                    max_len,
                )
            },
        )
        .map_err(|error| {
            PluginRuntimeError::new(
                "wasm_runtime",
                format!("define wasm file_policy_rules_match_dry_run hostcall failed: {error}"),
            )
        })?;
    linker
        .func_wrap(
            "actrail_host",
            "file_policy_rules_validate",
            |mut caller: Caller<'_, WasmStoreState>,
             patch_ptr: i32,
             patch_len: i32,
             out_ptr: i32,
             max_len: i32|
             -> i64 {
                file_policy_rules_apply_or_validate(
                    &mut caller,
                    patch_ptr,
                    patch_len,
                    out_ptr,
                    max_len,
                    false,
                )
            },
        )
        .map_err(|error| {
            PluginRuntimeError::new(
                "wasm_runtime",
                format!("define wasm file_policy_rules_validate hostcall failed: {error}"),
            )
        })?;
    linker
        .func_wrap(
            "actrail_host",
            "file_policy_rules_apply",
            |mut caller: Caller<'_, WasmStoreState>,
             patch_ptr: i32,
             patch_len: i32,
             out_ptr: i32,
             max_len: i32|
             -> i64 {
                file_policy_rules_apply_or_validate(
                    &mut caller,
                    patch_ptr,
                    patch_len,
                    out_ptr,
                    max_len,
                    true,
                )
            },
        )
        .map_err(|error| {
            PluginRuntimeError::new(
                "wasm_runtime",
                format!("define wasm file_policy_rules_apply hostcall failed: {error}"),
            )
        })?;
    Ok(linker)
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

fn env_read(
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

fn payload_read(
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

fn context_query(
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

fn file_access_current_match_get(
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

fn file_policy_rules_version_get(caller: Caller<'_, WasmStoreState>) -> i64 {
    if !can_access_file_policy_rules(caller.data()) {
        return FILE_POLICY_RULES_DENIED;
    }
    let Some(host) = caller.data().file_policy_host().cloned() else {
        return FILE_POLICY_RULES_NOT_FOUND;
    };
    match host.rules_version_get() {
        Ok(revision) => i64::try_from(revision).unwrap_or(FILE_POLICY_RULES_TOO_LARGE),
        Err(_) => FILE_POLICY_RULES_INVALID,
    }
}

#[allow(clippy::too_many_arguments)]
fn file_policy_rules_list(
    caller: &mut Caller<'_, WasmStoreState>,
    filter_ptr: i32,
    filter_len: i32,
    cursor_ptr: i32,
    cursor_len: i32,
    limit: i32,
    out_ptr: i32,
    max_len: i32,
) -> i64 {
    if !caller.data().host_grants().can_read_file_policy_rules() {
        return FILE_POLICY_RULES_DENIED;
    }
    let Some(host) = caller.data().file_policy_host().cloned() else {
        return FILE_POLICY_RULES_NOT_FOUND;
    };
    let Ok(memory) = exported_memory(caller) else {
        return FILE_POLICY_RULES_INVALID;
    };
    let Ok(filter) = read_guest_bytes(caller, &memory, filter_ptr, filter_len) else {
        return FILE_POLICY_RULES_INVALID;
    };
    if filter.len() > caller.data().host_limits().file_policy_io_max_bytes {
        return FILE_POLICY_RULES_TOO_LARGE;
    }
    let Ok(cursor_bytes) = read_guest_bytes(caller, &memory, cursor_ptr, cursor_len) else {
        return FILE_POLICY_RULES_INVALID;
    };
    if cursor_bytes.len() > caller.data().host_limits().file_policy_io_max_bytes {
        return FILE_POLICY_RULES_TOO_LARGE;
    }
    let filter = match parse_file_policy_list_filter(&filter) {
        Ok(filter) => filter,
        Err(_) => return FILE_POLICY_RULES_INVALID,
    };
    let cursor = if cursor_bytes.is_empty() {
        None
    } else {
        match String::from_utf8(cursor_bytes) {
            Ok(cursor) => Some(cursor),
            Err(_) => return FILE_POLICY_RULES_INVALID,
        }
    };
    let limit = match u32::try_from(limit) {
        Ok(limit) => limit,
        Err(_) => return FILE_POLICY_RULES_INVALID,
    };
    let result = match host.rules_list(filter, cursor, limit) {
        Ok(result) => result,
        Err(_) => return FILE_POLICY_RULES_INVALID,
    };
    write_guest_response(
        caller,
        &memory,
        out_ptr,
        max_len,
        &encode_file_policy_list_result(&result),
    )
}

fn file_policy_rules_match_dry_run(
    caller: &mut Caller<'_, WasmStoreState>,
    request_ptr: i32,
    request_len: i32,
    out_ptr: i32,
    max_len: i32,
) -> i64 {
    if !caller
        .data()
        .host_grants()
        .can_match_dry_run_file_policy_rules()
    {
        return FILE_POLICY_RULES_DENIED;
    }
    let Some(host) = caller.data().file_policy_host().cloned() else {
        return FILE_POLICY_RULES_NOT_FOUND;
    };
    let Ok(memory) = exported_memory(caller) else {
        return FILE_POLICY_RULES_INVALID;
    };
    let Ok(request) = read_guest_bytes(caller, &memory, request_ptr, request_len) else {
        return FILE_POLICY_RULES_INVALID;
    };
    if request.len() > caller.data().host_limits().file_policy_io_max_bytes {
        return FILE_POLICY_RULES_TOO_LARGE;
    }
    let request = match parse_file_policy_match_dry_run_request(&request) {
        Ok(request) => request,
        Err(_) => return FILE_POLICY_RULES_INVALID,
    };
    let result = match host.rules_match_dry_run(request) {
        Ok(result) => result,
        Err(_) => return FILE_POLICY_RULES_INVALID,
    };
    write_guest_response(
        caller,
        &memory,
        out_ptr,
        max_len,
        &encode_file_policy_match_dry_run_result(&result),
    )
}

fn file_policy_rules_apply_or_validate(
    caller: &mut Caller<'_, WasmStoreState>,
    patch_ptr: i32,
    patch_len: i32,
    out_ptr: i32,
    max_len: i32,
    apply: bool,
) -> i64 {
    if apply && !caller.data().host_grants().can_apply_file_policy_rules() {
        return FILE_POLICY_RULES_DENIED;
    }
    if !apply && !caller.data().host_grants().can_validate_file_policy_rules() {
        return FILE_POLICY_RULES_DENIED;
    }
    let Some(host) = caller.data().file_policy_host().cloned() else {
        return FILE_POLICY_RULES_NOT_FOUND;
    };
    let Some(owner) = caller
        .data()
        .file_policy_owner_instance_id()
        .map(str::to_string)
    else {
        return FILE_POLICY_RULES_NOT_FOUND;
    };
    let grants = caller
        .data()
        .host_grants()
        .file_policy_rules_apply_grants()
        .to_vec();
    let Ok(memory) = exported_memory(caller) else {
        return FILE_POLICY_RULES_INVALID;
    };
    let Ok((patch_offset, patch_len)) = guest_range(patch_ptr, patch_len) else {
        return FILE_POLICY_RULES_INVALID;
    };
    if patch_len > caller.data().host_limits().file_policy_io_max_bytes {
        return FILE_POLICY_RULES_TOO_LARGE;
    }
    let Ok((out_offset, max_len)) = guest_range(out_ptr, max_len) else {
        return FILE_POLICY_RULES_INVALID;
    };
    if max_len > caller.data().host_limits().file_policy_io_max_bytes {
        return FILE_POLICY_RULES_TOO_LARGE;
    }
    let mut patch = vec![0_u8; patch_len];
    if memory.read(&mut *caller, patch_offset, &mut patch).is_err() {
        return FILE_POLICY_RULES_INVALID;
    }
    let request = match parse_file_policy_apply_request(&patch) {
        Ok(request) => request,
        Err(_) => return FILE_POLICY_RULES_INVALID,
    };
    let result = if apply {
        host.rules_apply(&owner, &grants, request)
    } else {
        host.rules_validate(&owner, &grants, &request)
    };
    let result = match result {
        Ok(result) => result,
        Err(_) => return FILE_POLICY_RULES_REJECTED,
    };
    let response = encode_file_policy_apply_result(&result);
    if response.len() > max_len {
        return FILE_POLICY_RULES_TOO_LARGE;
    }
    if memory.write(&mut *caller, out_offset, &response).is_err() {
        return FILE_POLICY_RULES_INVALID;
    }
    i64::try_from(response.len()).unwrap_or(FILE_POLICY_RULES_TOO_LARGE)
}

fn can_access_file_policy_rules(state: &WasmStoreState) -> bool {
    state.host_grants().can_read_file_policy_rules()
        || state.host_grants().can_match_dry_run_file_policy_rules()
        || state.host_grants().can_validate_file_policy_rules()
        || state.host_grants().can_apply_file_policy_rules()
}

fn parse_file_policy_list_filter(bytes: &[u8]) -> Result<FilePolicyListFilter, String> {
    let mut cursor = BinaryCursor::new(bytes);
    let version = cursor.read_u8()?;
    if version != FILE_POLICY_RULES_BINARY_VERSION {
        return Err(format!("unsupported file policy binary version {version}"));
    }
    let decision = match cursor.read_u8()? {
        0 => None,
        _ => Some(FilePolicyDecision::from_code(cursor.read_u8()?)?),
    };
    let operation = match cursor.read_u8()? {
        0 => None,
        _ => Some(FilePolicyOperation::from_code(cursor.read_u8()?)?),
    };
    let path_prefix = cursor.read_string_u16()?;
    if !cursor.is_empty() {
        return Err("file policy list filter has trailing bytes".to_string());
    }
    Ok(FilePolicyListFilter {
        decision,
        path_prefix,
        operation,
    })
}

fn parse_file_policy_match_dry_run_request(
    bytes: &[u8],
) -> Result<FilePolicyMatchDryRunRequest, String> {
    let mut cursor = BinaryCursor::new(bytes);
    let version = cursor.read_u8()?;
    if version != FILE_POLICY_RULES_BINARY_VERSION {
        return Err(format!("unsupported file policy binary version {version}"));
    }
    let operation = FilePolicyOperation::from_code(cursor.read_u8()?)?;
    let path = cursor
        .read_string_u16()?
        .ok_or_else(|| "file policy dry-run path is required".to_string())?;
    if !cursor.is_empty() {
        return Err("file policy dry-run request has trailing bytes".to_string());
    }
    Ok(FilePolicyMatchDryRunRequest { path, operation })
}

fn parse_file_policy_apply_request(bytes: &[u8]) -> Result<FilePolicyApplyRequest, String> {
    let mut cursor = BinaryCursor::new(bytes);
    let version = cursor.read_u8()?;
    if version != FILE_POLICY_RULES_BINARY_VERSION {
        return Err(format!("unsupported file policy binary version {version}"));
    }
    let apply_mode = FilePolicyApplyMode::from_code(cursor.read_u8()?)?;
    let base_revision = cursor.read_u64()?;
    let item_count = cursor.read_u32()?;
    let item_count = usize::try_from(item_count)
        .map_err(|error| format!("file policy item count overflow: {error}"))?;
    let mut items = Vec::with_capacity(item_count);
    for _ in 0..item_count {
        items.push(parse_file_policy_patch_item(&mut cursor)?);
    }
    if !cursor.is_empty() {
        return Err("file policy patch has trailing bytes".to_string());
    }
    Ok(FilePolicyApplyRequest {
        items,
        precondition: FilePolicyApplyPrecondition {
            base_revision,
            mutation_id: String::new(),
            reason: None,
            correlation_id: None,
            apply_mode,
        },
    })
}

fn parse_file_policy_patch_item(
    cursor: &mut BinaryCursor<'_>,
) -> Result<FilePolicyPatchItem, String> {
    let op = FilePolicyPatchOp::from_code(cursor.read_u8()?)?;
    let decision = FilePolicyDecision::from_code(cursor.read_u8()?)?;
    let operation = FilePolicyOperation::from_code(cursor.read_u8()?)?;
    let priority = cursor.read_i32()?;
    let gray_target = match cursor.read_u64()? {
        0 => None,
        value => Some(value),
    };
    let rule_id = cursor.read_string_u16()?;
    let path = cursor.read_string_u16()?;
    let rule = matches!(op, FilePolicyPatchOp::Upsert).then(|| FilePolicyRuleDraft {
        rule_id: rule_id.clone(),
        decision,
        operation,
        path: path.unwrap_or_default(),
        gray_target,
        priority,
    });
    Ok(FilePolicyPatchItem { op, rule_id, rule })
}

fn encode_file_policy_apply_result(result: &plugin_system::FilePolicyApplyResult) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.push(result.status.code());
    bytes.extend_from_slice(&result.new_revision.to_le_bytes());
    bytes.extend_from_slice(&result.applied_count.to_le_bytes());
    bytes.extend_from_slice(&result.rejected_count.to_le_bytes());
    bytes.extend_from_slice(&(result.errors.len() as u32).to_le_bytes());
    for error in &result.errors {
        bytes.extend_from_slice(&error.item_index.to_le_bytes());
        push_u16_bytes(&mut bytes, error.code.as_bytes());
        push_u16_bytes(&mut bytes, error.message.as_bytes());
    }
    bytes
}

fn encode_file_policy_list_result(result: &plugin_system::FilePolicyListResult) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&result.source_revision.to_le_bytes());
    push_u16_bytes(
        &mut bytes,
        result.next_cursor.as_deref().unwrap_or_default().as_bytes(),
    );
    bytes.extend_from_slice(&(result.rules.len() as u32).to_le_bytes());
    for rule in &result.rules {
        bytes.push(rule.decision.code());
        bytes.push(rule.operation.code());
        bytes.extend_from_slice(&rule.gray_target.unwrap_or_default().to_le_bytes());
        bytes.extend_from_slice(&rule.priority.to_le_bytes());
        bytes.push(u8::from(rule.enabled));
        bytes.extend_from_slice(&rule.updated_sequence.to_le_bytes());
        push_u16_bytes(&mut bytes, rule.rule_id.as_bytes());
        push_u16_bytes(&mut bytes, rule.owner_instance_id.as_bytes());
        push_u16_bytes(&mut bytes, rule.path.as_bytes());
    }
    bytes
}

fn encode_file_policy_match_dry_run_result(
    result: &plugin_system::FilePolicyMatchDryRunResult,
) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.push(u8::from(result.matched));
    bytes.push(result.decision.code());
    bytes.push(result.operation.code());
    bytes.extend_from_slice(&result.source_revision.to_le_bytes());
    push_u16_bytes(
        &mut bytes,
        result.rule_id.as_deref().unwrap_or_default().as_bytes(),
    );
    push_u16_bytes(&mut bytes, result.canonical_path.as_bytes());
    bytes
}

fn push_u16_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    let len = u16::try_from(bytes.len()).unwrap_or(u16::MAX);
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&bytes[..usize::from(len)]);
}

struct BinaryCursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> BinaryCursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn is_empty(&self) -> bool {
        self.offset == self.bytes.len()
    }

    fn read_u8(&mut self) -> Result<u8, String> {
        let bytes = self.read_exact(1)?;
        Ok(bytes[0])
    }

    fn read_u16(&mut self) -> Result<u16, String> {
        let bytes = self.read_exact(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    fn read_u32(&mut self) -> Result<u32, String> {
        let bytes = self.read_exact(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn read_u64(&mut self) -> Result<u64, String> {
        let bytes = self.read_exact(8)?;
        Ok(u64::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }

    fn read_i32(&mut self) -> Result<i32, String> {
        let bytes = self.read_exact(4)?;
        Ok(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn read_string_u16(&mut self) -> Result<Option<String>, String> {
        let len = usize::from(self.read_u16()?);
        if len == 0 {
            return Ok(None);
        }
        let bytes = self.read_exact(len)?;
        let value = std::str::from_utf8(bytes)
            .map_err(|error| format!("file policy string is not utf-8: {error}"))?;
        Ok(Some(value.to_string()))
    }

    fn read_exact(&mut self, len: usize) -> Result<&'a [u8], String> {
        let end = self
            .offset
            .checked_add(len)
            .ok_or_else(|| "file policy binary offset overflow".to_string())?;
        if end > self.bytes.len() {
            return Err("file policy binary payload is truncated".to_string());
        }
        let bytes = &self.bytes[self.offset..end];
        self.offset = end;
        Ok(bytes)
    }
}

pub(crate) fn matched_rule_response(context: &FilePolicyReadContext) -> String {
    let rule = &context.matched_rule;
    let mut response = String::new();
    push_context_field(
        &mut response,
        legacy_policy_text::field::VERSION,
        legacy_policy_text::CURRENT_MATCH_SCHEMA_VERSION,
    );
    push_context_field(
        &mut response,
        legacy_policy_text::field::RULE_ID,
        &rule.rule_id,
    );
    push_context_field(
        &mut response,
        legacy_policy_text::field::DECISION,
        &rule.decision,
    );
    if let Some(fallback) = &rule.fallback {
        push_context_field(&mut response, legacy_policy_text::field::FALLBACK, fallback);
    }
    if let Some(timeout_ms) = rule.timeout_ms {
        push_context_field(
            &mut response,
            legacy_policy_text::field::TIMEOUT_MS,
            &timeout_ms.to_string(),
        );
    }
    if let Some(concurrency_limit) = rule.concurrency_limit {
        push_context_field(
            &mut response,
            legacy_policy_text::field::CONCURRENCY_LIMIT,
            &concurrency_limit.to_string(),
        );
    }
    push_context_field(
        &mut response,
        legacy_policy_text::field::OPERATION,
        &rule.operation,
    );
    if let Some(plugin_instance) = &rule.plugin_instance {
        push_context_field(
            &mut response,
            legacy_policy_text::field::PLUGIN_INSTANCE,
            plugin_instance,
        );
    }
    push_context_field(&mut response, legacy_policy_text::field::PATH, &rule.path);
    response
}

pub(crate) fn decision_summary_response(context: &ControlContextSnapshot) -> String {
    let mut response = String::new();
    push_context_field(
        &mut response,
        legacy_policy_text::field::VERSION,
        legacy_policy_text::CONTEXT_QUERY_SCHEMA_VERSION,
    );
    push_context_field(
        &mut response,
        legacy_policy_text::field::SUBJECT,
        &context.subject,
    );
    push_context_field(
        &mut response,
        legacy_policy_text::field::OPERATION,
        &context.operation,
    );
    push_context_field(
        &mut response,
        legacy_policy_text::field::TARGET_SUMMARY,
        &context.target_summary,
    );
    push_context_field(
        &mut response,
        legacy_policy_text::field::DECISION_ID,
        &context.decision_id,
    );
    push_context_field(
        &mut response,
        legacy_policy_text::field::TRACE_ID,
        &context.trace_id,
    );
    push_context_field(
        &mut response,
        legacy_policy_text::field::ACTOR_PROCESS_IDENTITY,
        &context.actor_process_identity,
    );
    response
}

fn push_context_field(response: &mut String, key: &str, value: &str) {
    response.push_str(key);
    response.push('=');
    for ch in value.chars() {
        match ch {
            '\\' => response.push_str("\\\\"),
            '\n' => response.push_str("\\n"),
            '\r' => response.push_str("\\r"),
            other => response.push(other),
        }
    }
    response.push('\n');
}

fn exported_memory(caller: &mut Caller<'_, WasmStoreState>) -> Result<Memory, ()> {
    caller
        .get_export("memory")
        .and_then(|export| export.into_memory())
        .ok_or(())
}

fn read_guest_bytes(
    caller: &mut Caller<'_, WasmStoreState>,
    memory: &Memory,
    ptr: i32,
    len: i32,
) -> Result<Vec<u8>, ()> {
    let (offset, len) = guest_range(ptr, len)?;
    let mut bytes = vec![0_u8; len];
    memory.read(caller, offset, &mut bytes).map_err(|_| ())?;
    Ok(bytes)
}

fn write_guest_response(
    caller: &mut Caller<'_, WasmStoreState>,
    memory: &Memory,
    out_ptr: i32,
    max_len: i32,
    response: &[u8],
) -> i64 {
    let Ok((out_offset, max_len)) = guest_range(out_ptr, max_len) else {
        return FILE_POLICY_RULES_INVALID;
    };
    if max_len > caller.data().host_limits().file_policy_io_max_bytes {
        return FILE_POLICY_RULES_TOO_LARGE;
    }
    if response.len() > max_len {
        return FILE_POLICY_RULES_TOO_LARGE;
    }
    if memory.write(caller, out_offset, response).is_err() {
        return FILE_POLICY_RULES_INVALID;
    }
    i64::try_from(response.len()).unwrap_or(FILE_POLICY_RULES_TOO_LARGE)
}

fn guest_range(ptr: i32, len: i32) -> Result<(usize, usize), ()> {
    if ptr < 0 || len < 0 {
        return Err(());
    }
    let offset = usize::try_from(ptr).map_err(|_| ())?;
    let len = usize::try_from(len).map_err(|_| ())?;
    offset.checked_add(len).ok_or(())?;
    Ok((offset, len))
}
