//! WIT component context and dynamic-policy hostcall handlers.

use plugin_system::{
    COMMAND_EXECUTION_CONTEXT_QUERY, COMMAND_EXECUTION_CURRENT_CONTEXT_TOKEN,
    CONTROL_DECISION_SUMMARY_QUERY, FILE_POLICY_MATCHED_RULE_QUERY,
};
use wasmtime::component::Val;

use crate::engine::WasmStoreState;

use super::command_codec::{CommandContextWireCodec, CommandPolicyWireCodec};
use super::file_codec::{
    component_file_policy_apply_result, component_file_policy_list_result,
    component_file_policy_match_dry_run_result, parse_component_file_policy_apply_request,
    parse_component_file_policy_list_filter, parse_component_file_policy_match_dry_run_request,
};
use super::value::{
    decision_summary_val, matched_rule_val, parse_component_option_string_val,
    set_component_string_error, set_component_val_ok,
};

pub(super) fn component_query_context(
    store: wasmtime::StoreContextMut<'_, WasmStoreState>,
    params: &[Val],
    results: &mut [Val],
) {
    if !store.data().host_grants().can_query_context() {
        set_component_string_error(results, "denied");
        return;
    }
    let [Val::String(context_ref), Val::String(query)] = params else {
        set_component_string_error(results, "invalid");
        return;
    };
    if context_ref.len() > store.data().host_limits().context_ref_max_bytes
        || query.len() > store.data().host_limits().context_query_max_bytes
    {
        set_component_string_error(results, "too-large");
        return;
    }
    let Some(context) = store.data().control_context() else {
        set_component_string_error(results, "not-found");
        return;
    };
    if context.context_ref != *context_ref || query != CONTROL_DECISION_SUMMARY_QUERY {
        set_component_string_error(results, "not-found");
        return;
    }
    set_component_val_ok(results, decision_summary_val(context));
}

pub(super) fn component_file_access_current_match_get(
    store: wasmtime::StoreContextMut<'_, WasmStoreState>,
    params: &[Val],
    results: &mut [Val],
) {
    if !store
        .data()
        .host_grants()
        .can_get_current_file_access_match()
    {
        set_component_string_error(results, "denied");
        return;
    }
    let [Val::String(context_ref), Val::String(query)] = params else {
        set_component_string_error(results, "invalid");
        return;
    };
    if context_ref.len() > store.data().host_limits().file_policy_context_ref_max_bytes
        || query.len() > store.data().host_limits().file_policy_query_max_bytes
    {
        set_component_string_error(results, "too-large");
        return;
    }
    let Some(context) = store.data().file_policy_context() else {
        set_component_string_error(results, "not-found");
        return;
    };
    if context.context_ref != *context_ref || query != FILE_POLICY_MATCHED_RULE_QUERY {
        set_component_string_error(results, "not-found");
        return;
    }
    set_component_val_ok(results, matched_rule_val(context));
}

pub(super) fn component_file_policy_rules_version_get(
    store: wasmtime::StoreContextMut<'_, WasmStoreState>,
    results: &mut [Val],
) {
    if !can_access_file_policy_rules(store.data()) {
        set_component_string_error(results, "denied");
        return;
    }
    let Some(host) = store.data().file_policy_host().cloned() else {
        set_component_string_error(results, "not-found");
        return;
    };
    match host.rules_version_get() {
        Ok(revision) => set_component_val_ok(results, Val::U64(revision)),
        Err(error) => set_component_string_error(results, &error.message),
    }
}

pub(super) fn component_file_policy_rules_list(
    store: wasmtime::StoreContextMut<'_, WasmStoreState>,
    params: &[Val],
    results: &mut [Val],
) {
    if !store.data().host_grants().can_read_file_policy_rules() {
        set_component_string_error(results, "denied");
        return;
    }
    let [Val::Record(filter_fields), cursor, Val::U32(limit)] = params else {
        set_component_string_error(results, "invalid");
        return;
    };
    let cursor = match parse_component_option_string_val(cursor) {
        Ok(cursor) => cursor,
        Err(error) => {
            set_component_string_error(results, &error);
            return;
        }
    };
    let filter = match parse_component_file_policy_list_filter(filter_fields) {
        Ok(filter) => filter,
        Err(error) => {
            set_component_string_error(results, &error);
            return;
        }
    };
    let Some(host) = store.data().file_policy_host().cloned() else {
        set_component_string_error(results, "not-found");
        return;
    };
    match host.rules_list(filter, cursor, *limit) {
        Ok(result) => set_component_val_ok(results, component_file_policy_list_result(result)),
        Err(error) => set_component_string_error(results, &error.message),
    }
}

pub(super) fn component_file_policy_rules_match_dry_run(
    store: wasmtime::StoreContextMut<'_, WasmStoreState>,
    params: &[Val],
    results: &mut [Val],
) {
    if !store
        .data()
        .host_grants()
        .can_match_dry_run_file_policy_rules()
    {
        set_component_string_error(results, "denied");
        return;
    }
    let [Val::Record(fields)] = params else {
        set_component_string_error(results, "invalid");
        return;
    };
    let request = match parse_component_file_policy_match_dry_run_request(fields) {
        Ok(request) => request,
        Err(error) => {
            set_component_string_error(results, &error);
            return;
        }
    };
    let Some(host) = store.data().file_policy_host().cloned() else {
        set_component_string_error(results, "not-found");
        return;
    };
    match host.rules_match_dry_run(request) {
        Ok(result) => {
            set_component_val_ok(results, component_file_policy_match_dry_run_result(result))
        }
        Err(error) => set_component_string_error(results, &error.message),
    }
}

pub(super) fn component_file_policy_rules_apply_or_validate(
    store: wasmtime::StoreContextMut<'_, WasmStoreState>,
    params: &[Val],
    results: &mut [Val],
    apply: bool,
) {
    if apply && !store.data().host_grants().can_apply_file_policy_rules() {
        set_component_string_error(results, "denied");
        return;
    }
    if !apply && !store.data().host_grants().can_validate_file_policy_rules() {
        set_component_string_error(results, "denied");
        return;
    }
    let [Val::Record(fields)] = params else {
        set_component_string_error(results, "invalid");
        return;
    };
    let Some(host) = store.data().file_policy_host().cloned() else {
        set_component_string_error(results, "not-found");
        return;
    };
    let Some(owner) = store
        .data()
        .file_policy_owner_instance_id()
        .map(str::to_string)
    else {
        set_component_string_error(results, "not-found");
        return;
    };
    let grants = store
        .data()
        .host_grants()
        .file_policy_rules_apply_grants()
        .to_vec();
    let request = match parse_component_file_policy_apply_request(fields) {
        Ok(request) => request,
        Err(error) => {
            set_component_string_error(results, &error);
            return;
        }
    };
    let result = if apply {
        host.rules_apply(&owner, &grants, request)
    } else {
        host.rules_validate(&owner, &grants, &request)
    };
    match result {
        Ok(result) => set_component_val_ok(results, component_file_policy_apply_result(result)),
        Err(error) => set_component_string_error(results, &error.message),
    }
}

fn can_access_file_policy_rules(state: &WasmStoreState) -> bool {
    state.host_grants().can_read_file_policy_rules()
        || state.host_grants().can_match_dry_run_file_policy_rules()
        || state.host_grants().can_validate_file_policy_rules()
        || state.host_grants().can_apply_file_policy_rules()
}

pub(super) fn component_command_execution_context_query(
    store: wasmtime::StoreContextMut<'_, WasmStoreState>,
    params: &[Val],
    results: &mut [Val],
) {
    if !store
        .data()
        .host_grants()
        .can_query_current_command_execution_context()
    {
        set_component_string_error(results, "denied");
        return;
    }
    let [Val::String(context_ref), Val::String(query)] = params else {
        set_component_string_error(results, "invalid");
        return;
    };
    if context_ref.len() > store.data().host_limits().command_context_ref_max_bytes
        || query.len() > store.data().host_limits().command_context_query_max_bytes
    {
        set_component_string_error(results, "too-large");
        return;
    }
    if context_ref != COMMAND_EXECUTION_CURRENT_CONTEXT_TOKEN
        || query != COMMAND_EXECUTION_CONTEXT_QUERY
    {
        set_component_string_error(results, "not-found");
        return;
    }
    let Some(context) = store.data().command_execution_context() else {
        set_component_string_error(results, "not-found");
        return;
    };
    if CommandContextWireCodec::size(context)
        > store.data().host_limits().command_context_read_max_bytes
    {
        set_component_string_error(results, "too-large");
        return;
    }
    set_component_val_ok(results, CommandContextWireCodec::encode(context));
}

pub(super) fn component_command_policy_rules_version_get(
    store: wasmtime::StoreContextMut<'_, WasmStoreState>,
    results: &mut [Val],
) {
    if !can_access_command_policy_rules(store.data()) {
        set_component_string_error(results, "denied");
        return;
    }
    let Some(host) = store.data().command_policy_host().cloned() else {
        set_component_string_error(results, "not-found");
        return;
    };
    match host.rules_version_get() {
        Ok(revision) => set_component_val_ok(results, Val::U64(revision)),
        Err(error) => set_component_string_error(results, &error.message),
    }
}

pub(super) fn component_command_policy_rules_list(
    store: wasmtime::StoreContextMut<'_, WasmStoreState>,
    params: &[Val],
    results: &mut [Val],
) {
    if !store.data().host_grants().can_read_command_policy_rules() {
        set_component_string_error(results, "denied");
        return;
    }
    let [Val::Record(filter_fields), cursor, Val::U32(limit)] = params else {
        set_component_string_error(results, "invalid");
        return;
    };
    let parsed = parse_component_option_string_val(cursor).and_then(|cursor| {
        CommandPolicyWireCodec::parse_list_filter(filter_fields).map(|filter| (filter, cursor))
    });
    let (filter, cursor) = match parsed {
        Ok(parsed) => parsed,
        Err(error) => {
            set_component_string_error(results, &error);
            return;
        }
    };
    let Some(host) = store.data().command_policy_host().cloned() else {
        set_component_string_error(results, "not-found");
        return;
    };
    match host.rules_list(filter, cursor, *limit) {
        Ok(result)
            if CommandPolicyWireCodec::list_result_size(&result)
                <= store.data().host_limits().command_policy_io_max_bytes =>
        {
            set_component_val_ok(results, CommandPolicyWireCodec::encode_list_result(result));
        }
        Ok(_) => set_component_string_error(results, "too-large"),
        Err(error) => set_component_string_error(results, &error.message),
    }
}

pub(super) fn component_command_policy_rules_match_dry_run(
    store: wasmtime::StoreContextMut<'_, WasmStoreState>,
    params: &[Val],
    results: &mut [Val],
) {
    if !store
        .data()
        .host_grants()
        .can_match_dry_run_command_policy_rules()
    {
        set_component_string_error(results, "denied");
        return;
    }
    let [Val::Record(fields)] = params else {
        set_component_string_error(results, "invalid");
        return;
    };
    let request = match CommandPolicyWireCodec::parse_match_request(fields) {
        Ok(request)
            if CommandPolicyWireCodec::match_request_size(&request)
                <= store.data().host_limits().command_policy_io_max_bytes =>
        {
            request
        }
        Ok(_) => {
            set_component_string_error(results, "too-large");
            return;
        }
        Err(error) => {
            set_component_string_error(results, &error);
            return;
        }
    };
    let Some(host) = store.data().command_policy_host().cloned() else {
        set_component_string_error(results, "not-found");
        return;
    };
    match host.rules_match_dry_run(request) {
        Ok(result)
            if CommandPolicyWireCodec::match_result_size(&result)
                <= store.data().host_limits().command_policy_io_max_bytes =>
        {
            set_component_val_ok(results, CommandPolicyWireCodec::encode_match_result(result));
        }
        Ok(_) => set_component_string_error(results, "too-large"),
        Err(error) => set_component_string_error(results, &error.message),
    }
}

pub(super) fn component_command_policy_rules_apply_or_validate(
    store: wasmtime::StoreContextMut<'_, WasmStoreState>,
    params: &[Val],
    results: &mut [Val],
    apply: bool,
) {
    let allowed = if apply {
        store.data().host_grants().can_apply_command_policy_rules()
    } else {
        store
            .data()
            .host_grants()
            .can_validate_command_policy_rules()
    };
    if !allowed {
        set_component_string_error(results, "denied");
        return;
    }
    let [Val::Record(fields)] = params else {
        set_component_string_error(results, "invalid");
        return;
    };
    let request = match CommandPolicyWireCodec::parse_apply_request(fields) {
        Ok(request)
            if CommandPolicyWireCodec::apply_request_size(&request)
                <= store.data().host_limits().command_policy_io_max_bytes =>
        {
            request
        }
        Ok(_) => {
            set_component_string_error(results, "too-large");
            return;
        }
        Err(error) => {
            set_component_string_error(results, &error);
            return;
        }
    };
    let Some(host) = store.data().command_policy_host().cloned() else {
        set_component_string_error(results, "not-found");
        return;
    };
    let Some(owner) = store
        .data()
        .command_policy_owner_instance_id()
        .map(str::to_string)
    else {
        set_component_string_error(results, "not-found");
        return;
    };
    let grants = store
        .data()
        .host_grants()
        .command_policy_rules_apply_grants()
        .to_vec();
    let result = if apply {
        host.rules_apply(&owner, &grants, request)
    } else {
        host.rules_validate(&owner, &grants, &request)
    };
    match result {
        Ok(result)
            if CommandPolicyWireCodec::apply_result_size(&result)
                <= store.data().host_limits().command_policy_io_max_bytes =>
        {
            set_component_val_ok(results, CommandPolicyWireCodec::encode_apply_result(result));
        }
        Ok(_) => set_component_string_error(results, "too-large"),
        Err(error) => set_component_string_error(results, &error.message),
    }
}

fn can_access_command_policy_rules(state: &WasmStoreState) -> bool {
    state.host_grants().can_read_command_policy_rules()
        || state.host_grants().can_match_dry_run_command_policy_rules()
        || state.host_grants().can_validate_command_policy_rules()
        || state.host_grants().can_apply_command_policy_rules()
}
