//! Legacy core-module hostcall linker and shared response/memory primitives.

use plugin_system::{FilePolicyReadContext, PluginRuntimeError};
use wasmtime::{Caller, Engine, Linker, Memory};

use crate::engine::{ControlContextSnapshot, WasmStoreState};

#[path = "host/command.rs"]
mod command;
#[path = "host/data_access.rs"]
mod data_access;
#[path = "host/file_policy.rs"]
mod file_policy;

pub(crate) use data_access::component_read_config;
use data_access::{context_query, env_read, file_access_current_match_get, payload_read};
use file_policy::{
    file_policy_rules_apply_or_validate, file_policy_rules_list, file_policy_rules_match_dry_run,
    file_policy_rules_version_get,
};

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
    command::LegacyCommandHostcalls::define(&mut linker)?;
    Ok(linker)
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

fn guest_range(ptr: i32, len: i32) -> Result<(usize, usize), ()> {
    if ptr < 0 || len < 0 {
        return Err(());
    }
    let offset = usize::try_from(ptr).map_err(|_| ())?;
    let len = usize::try_from(len).map_err(|_| ())?;
    offset.checked_add(len).ok_or(())?;
    Ok((offset, len))
}
