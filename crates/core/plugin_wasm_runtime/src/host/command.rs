//! Legacy core-module command context and dynamic policy hostcalls.

use plugin_system::{
    COMMAND_EXECUTION_CONTEXT_QUERY, COMMAND_EXECUTION_CURRENT_CONTEXT_TOKEN, PluginRuntimeError,
};
use wasmtime::{Caller, Linker, Memory};

use crate::engine::WasmStoreState;

use super::{exported_memory, guest_range};

#[path = "command/codec.rs"]
mod codec;

use codec::CommandPolicyBinaryCodec;

const DENIED: i64 = -1;
const NOT_FOUND: i64 = -2;
const INVALID: i64 = -3;
const TOO_LARGE: i64 = -4;
const REJECTED: i64 = -5;

pub(super) struct LegacyCommandHostcalls;

impl LegacyCommandHostcalls {
    pub(super) fn define(linker: &mut Linker<WasmStoreState>) -> Result<(), PluginRuntimeError> {
        linker
            .func_wrap(
                "actrail_host",
                "command_execution_current_context_query",
                |mut caller: Caller<'_, WasmStoreState>,
                 context_ptr: i32,
                 context_len: i32,
                 query_ptr: i32,
                 query_len: i32,
                 out_ptr: i32,
                 max_len: i32|
                 -> i64 {
                    Self::context_query(
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
            .map_err(|error| Self::link_error("command_execution_current_context_query", error))?;
        linker
            .func_wrap(
                "actrail_host",
                "command_policy_rules_version_get",
                |caller: Caller<'_, WasmStoreState>| -> i64 { Self::rules_version_get(caller) },
            )
            .map_err(|error| Self::link_error("command_policy_rules_version_get", error))?;
        linker
            .func_wrap(
                "actrail_host",
                "command_policy_rules_list",
                |mut caller: Caller<'_, WasmStoreState>,
                 filter_ptr: i32,
                 filter_len: i32,
                 cursor_ptr: i32,
                 cursor_len: i32,
                 limit: i32,
                 out_ptr: i32,
                 max_len: i32|
                 -> i64 {
                    Self::rules_list(
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
            .map_err(|error| Self::link_error("command_policy_rules_list", error))?;
        linker
            .func_wrap(
                "actrail_host",
                "command_policy_rules_match_dry_run",
                |mut caller: Caller<'_, WasmStoreState>,
                 request_ptr: i32,
                 request_len: i32,
                 out_ptr: i32,
                 max_len: i32|
                 -> i64 {
                    Self::rules_match_dry_run(
                        &mut caller,
                        request_ptr,
                        request_len,
                        out_ptr,
                        max_len,
                    )
                },
            )
            .map_err(|error| Self::link_error("command_policy_rules_match_dry_run", error))?;
        for (name, apply) in [
            ("command_policy_rules_validate", false),
            ("command_policy_rules_apply", true),
        ] {
            linker
                .func_wrap(
                    "actrail_host",
                    name,
                    move |mut caller: Caller<'_, WasmStoreState>,
                          request_ptr: i32,
                          request_len: i32,
                          out_ptr: i32,
                          max_len: i32|
                          -> i64 {
                        Self::rules_apply_or_validate(
                            &mut caller,
                            request_ptr,
                            request_len,
                            out_ptr,
                            max_len,
                            apply,
                        )
                    },
                )
                .map_err(|error| Self::link_error(name, error))?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn context_query(
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
            .can_query_current_command_execution_context()
        {
            return DENIED;
        }
        let Ok(memory) = exported_memory(caller) else {
            return INVALID;
        };
        let limits = caller.data().host_limits().clone();
        let Ok(context) = Self::read_bounded(
            caller,
            &memory,
            context_ptr,
            context_len,
            limits.command_context_ref_max_bytes,
        ) else {
            return TOO_LARGE;
        };
        let Ok(query) = Self::read_bounded(
            caller,
            &memory,
            query_ptr,
            query_len,
            limits.command_context_query_max_bytes,
        ) else {
            return TOO_LARGE;
        };
        let (Ok(context), Ok(query)) = (std::str::from_utf8(&context), std::str::from_utf8(&query))
        else {
            return INVALID;
        };
        if context != COMMAND_EXECUTION_CURRENT_CONTEXT_TOKEN
            || query != COMMAND_EXECUTION_CONTEXT_QUERY
        {
            return NOT_FOUND;
        }
        let Some(context) = caller.data().command_execution_context() else {
            return NOT_FOUND;
        };
        let Ok(response) = CommandPolicyBinaryCodec::encode_context(context) else {
            return TOO_LARGE;
        };
        Self::write_response(
            caller,
            &memory,
            out_ptr,
            max_len,
            limits.command_context_read_max_bytes,
            &response,
        )
    }

    fn rules_version_get(caller: Caller<'_, WasmStoreState>) -> i64 {
        if !Self::can_access_rules(caller.data()) {
            return DENIED;
        }
        let Some(host) = caller.data().command_policy_host() else {
            return NOT_FOUND;
        };
        host.rules_version_get()
            .ok()
            .and_then(|revision| i64::try_from(revision).ok())
            .unwrap_or(INVALID)
    }

    #[allow(clippy::too_many_arguments)]
    fn rules_list(
        caller: &mut Caller<'_, WasmStoreState>,
        filter_ptr: i32,
        filter_len: i32,
        cursor_ptr: i32,
        cursor_len: i32,
        limit: i32,
        out_ptr: i32,
        max_len: i32,
    ) -> i64 {
        if !caller.data().host_grants().can_read_command_policy_rules() {
            return DENIED;
        }
        let Some(host) = caller.data().command_policy_host().cloned() else {
            return NOT_FOUND;
        };
        let Ok(memory) = exported_memory(caller) else {
            return INVALID;
        };
        let io_max = caller.data().host_limits().command_policy_io_max_bytes;
        let Ok(filter) = Self::read_bounded(caller, &memory, filter_ptr, filter_len, io_max) else {
            return TOO_LARGE;
        };
        let Ok(cursor) = Self::read_bounded(caller, &memory, cursor_ptr, cursor_len, io_max) else {
            return TOO_LARGE;
        };
        let Ok(filter) = CommandPolicyBinaryCodec::parse_list_filter(&filter) else {
            return INVALID;
        };
        let cursor = if cursor.is_empty() {
            None
        } else {
            match String::from_utf8(cursor) {
                Ok(cursor) => Some(cursor),
                Err(_) => return INVALID,
            }
        };
        let Ok(limit) = u32::try_from(limit) else {
            return INVALID;
        };
        let Ok(result) = host.rules_list(filter, cursor, limit) else {
            return INVALID;
        };
        let Ok(response) = CommandPolicyBinaryCodec::encode_list_result(&result) else {
            return TOO_LARGE;
        };
        Self::write_response(caller, &memory, out_ptr, max_len, io_max, &response)
    }

    fn rules_match_dry_run(
        caller: &mut Caller<'_, WasmStoreState>,
        request_ptr: i32,
        request_len: i32,
        out_ptr: i32,
        max_len: i32,
    ) -> i64 {
        if !caller
            .data()
            .host_grants()
            .can_match_dry_run_command_policy_rules()
        {
            return DENIED;
        }
        let Some(host) = caller.data().command_policy_host().cloned() else {
            return NOT_FOUND;
        };
        let Ok(memory) = exported_memory(caller) else {
            return INVALID;
        };
        let io_max = caller.data().host_limits().command_policy_io_max_bytes;
        let Ok(request) = Self::read_bounded(caller, &memory, request_ptr, request_len, io_max)
        else {
            return TOO_LARGE;
        };
        let Ok(request) = CommandPolicyBinaryCodec::parse_match_request(&request) else {
            return INVALID;
        };
        let Ok(result) = host.rules_match_dry_run(request) else {
            return INVALID;
        };
        let Ok(response) = CommandPolicyBinaryCodec::encode_match_result(&result) else {
            return TOO_LARGE;
        };
        Self::write_response(caller, &memory, out_ptr, max_len, io_max, &response)
    }

    fn rules_apply_or_validate(
        caller: &mut Caller<'_, WasmStoreState>,
        request_ptr: i32,
        request_len: i32,
        out_ptr: i32,
        max_len: i32,
        apply: bool,
    ) -> i64 {
        let grants = caller.data().host_grants();
        if (apply && !grants.can_apply_command_policy_rules())
            || (!apply && !grants.can_validate_command_policy_rules())
        {
            return DENIED;
        }
        let Some(host) = caller.data().command_policy_host().cloned() else {
            return NOT_FOUND;
        };
        let Some(owner) = caller
            .data()
            .command_policy_owner_instance_id()
            .map(str::to_string)
        else {
            return NOT_FOUND;
        };
        let grants = grants.command_policy_rules_apply_grants().to_vec();
        let Ok(memory) = exported_memory(caller) else {
            return INVALID;
        };
        let io_max = caller.data().host_limits().command_policy_io_max_bytes;
        let Ok(request) = Self::read_bounded(caller, &memory, request_ptr, request_len, io_max)
        else {
            return TOO_LARGE;
        };
        let Ok(request) = CommandPolicyBinaryCodec::parse_apply_request(&request) else {
            return INVALID;
        };
        let result = if apply {
            host.rules_apply(&owner, &grants, request)
        } else {
            host.rules_validate(&owner, &grants, &request)
        };
        let Ok(result) = result else {
            return REJECTED;
        };
        let Ok(response) = CommandPolicyBinaryCodec::encode_apply_result(&result) else {
            return TOO_LARGE;
        };
        Self::write_response(caller, &memory, out_ptr, max_len, io_max, &response)
    }

    fn can_access_rules(state: &WasmStoreState) -> bool {
        state.host_grants().can_read_command_policy_rules()
            || state.host_grants().can_match_dry_run_command_policy_rules()
            || state.host_grants().can_validate_command_policy_rules()
            || state.host_grants().can_apply_command_policy_rules()
    }

    fn read_bounded(
        caller: &mut Caller<'_, WasmStoreState>,
        memory: &Memory,
        ptr: i32,
        len: i32,
        max_len: usize,
    ) -> Result<Vec<u8>, ()> {
        let (offset, len) = guest_range(ptr, len)?;
        if len > max_len {
            return Err(());
        }
        let mut bytes = vec![0_u8; len];
        memory.read(caller, offset, &mut bytes).map_err(|_| ())?;
        Ok(bytes)
    }

    fn write_response(
        caller: &mut Caller<'_, WasmStoreState>,
        memory: &Memory,
        out_ptr: i32,
        max_len: i32,
        host_max_len: usize,
        response: &[u8],
    ) -> i64 {
        let Ok((offset, guest_max_len)) = guest_range(out_ptr, max_len) else {
            return INVALID;
        };
        if guest_max_len > host_max_len || response.len() > guest_max_len {
            return TOO_LARGE;
        }
        if memory.write(caller, offset, response).is_err() {
            return INVALID;
        }
        i64::try_from(response.len()).unwrap_or(TOO_LARGE)
    }

    fn link_error(name: &str, error: wasmtime::Error) -> PluginRuntimeError {
        PluginRuntimeError::new(
            "wasm_runtime",
            format!("define wasm {name} hostcall failed: {error}"),
        )
    }
}
