//! Typed network-action context and managed network-policy component host.

use plugin_system::NETWORK_ACTION_CONTEXT_QUERY;
use wasmtime::component::{Linker as ComponentLinker, Val};

use crate::engine::WasmStoreState;

use super::super::component_abi;
use super::super::value::{
    parse_component_option_string_val, set_component_string_error, set_component_val_ok,
};
use super::codec::NetworkComponentCodec;

pub(in crate::component_control) struct NetworkComponentHost;

impl NetworkComponentHost {
    pub(in crate::component_control) fn add_to(
        linker: &mut ComponentLinker<WasmStoreState>,
    ) -> Result<(), plugin_system::PluginRuntimeError> {
        let mut host = linker
            .instance(component_abi::NETWORK_CONTROL_HOST_IMPORT)
            .map_err(Self::link_error)?;
        host.func_new(
            component_abi::network_host_import::NETWORK_ACTION_CURRENT_CONTEXT_QUERY,
            |store, _ty, params, results| {
                Self::query_current_action(store, params, results);
                Ok(())
            },
        )
        .map_err(Self::link_error)?;
        host.func_new(
            component_abi::network_host_import::NETWORK_POLICY_RULES_VERSION_GET,
            |store, _ty, _params, results| {
                Self::rules_version_get(store, results);
                Ok(())
            },
        )
        .map_err(Self::link_error)?;
        host.func_new(
            component_abi::network_host_import::NETWORK_POLICY_RULES_LIST,
            |store, _ty, params, results| {
                Self::rules_list(store, params, results);
                Ok(())
            },
        )
        .map_err(Self::link_error)?;
        host.func_new(
            component_abi::network_host_import::NETWORK_POLICY_RULES_MATCH_DRY_RUN,
            |store, _ty, params, results| {
                Self::rules_match_dry_run(store, params, results);
                Ok(())
            },
        )
        .map_err(Self::link_error)?;
        host.func_new(
            component_abi::network_host_import::NETWORK_POLICY_RULES_VALIDATE,
            |store, _ty, params, results| {
                Self::rules_apply_or_validate(store, params, results, false);
                Ok(())
            },
        )
        .map_err(Self::link_error)?;
        host.func_new(
            component_abi::network_host_import::NETWORK_POLICY_RULES_APPLY,
            |store, _ty, params, results| {
                Self::rules_apply_or_validate(store, params, results, true);
                Ok(())
            },
        )
        .map_err(Self::link_error)?;
        Ok(())
    }

    fn query_current_action(
        store: wasmtime::StoreContextMut<'_, WasmStoreState>,
        params: &[Val],
        results: &mut [Val],
    ) {
        if !store
            .data()
            .host_grants()
            .can_query_current_network_action_context()
        {
            set_component_string_error(results, "denied");
            return;
        }
        let [Val::String(context_ref), Val::String(query)] = params else {
            set_component_string_error(results, "invalid");
            return;
        };
        if context_ref.len() > store.data().host_limits().network_context_ref_max_bytes
            || query.len() > store.data().host_limits().network_query_max_bytes
        {
            set_component_string_error(results, "too-large");
            return;
        }
        let Some(control_context) = store.data().control_context() else {
            set_component_string_error(results, "not-found");
            return;
        };
        let Some(context) = store.data().network_action_context() else {
            set_component_string_error(results, "not-found");
            return;
        };
        if control_context.context_ref != *context_ref || query != NETWORK_ACTION_CONTEXT_QUERY {
            set_component_string_error(results, "not-found");
            return;
        }
        if NetworkComponentCodec::context_size(context)
            > store.data().host_limits().network_policy_io_max_bytes
        {
            set_component_string_error(results, "too-large");
            return;
        }
        set_component_val_ok(results, NetworkComponentCodec::encode_context(context));
    }

    fn rules_version_get(
        store: wasmtime::StoreContextMut<'_, WasmStoreState>,
        results: &mut [Val],
    ) {
        if !Self::can_access_rules(store.data()) {
            set_component_string_error(results, "denied");
            return;
        }
        let Some(host) = store.data().network_policy_host().cloned() else {
            set_component_string_error(results, "not-found");
            return;
        };
        match host.rules_version_get() {
            Ok(revision) => set_component_val_ok(results, Val::U64(revision)),
            Err(error) => set_component_string_error(results, &error.message),
        }
    }

    fn rules_list(
        store: wasmtime::StoreContextMut<'_, WasmStoreState>,
        params: &[Val],
        results: &mut [Val],
    ) {
        if !store.data().host_grants().can_read_network_policy_rules() {
            set_component_string_error(results, "denied");
            return;
        }
        let [Val::Record(fields), cursor, Val::U32(limit)] = params else {
            set_component_string_error(results, "invalid");
            return;
        };
        let parsed = parse_component_option_string_val(cursor)
            .and_then(|cursor| Ok((NetworkComponentCodec::parse_list_filter(fields)?, cursor)));
        let (filter, cursor) = match parsed {
            Ok(parsed) => parsed,
            Err(error) => {
                set_component_string_error(results, &error);
                return;
            }
        };
        let Some(host) = store.data().network_policy_host().cloned() else {
            set_component_string_error(results, "not-found");
            return;
        };
        match host.rules_list(filter, cursor, *limit) {
            Ok(result)
                if NetworkComponentCodec::list_result_size(&result)
                    <= store.data().host_limits().network_policy_io_max_bytes =>
            {
                set_component_val_ok(results, NetworkComponentCodec::encode_list_result(result));
            }
            Ok(_) => set_component_string_error(results, "too-large"),
            Err(error) => set_component_string_error(results, &error.message),
        }
    }

    fn rules_match_dry_run(
        store: wasmtime::StoreContextMut<'_, WasmStoreState>,
        params: &[Val],
        results: &mut [Val],
    ) {
        if !store
            .data()
            .host_grants()
            .can_match_dry_run_network_policy_rules()
        {
            set_component_string_error(results, "denied");
            return;
        }
        let [Val::Record(fields)] = params else {
            set_component_string_error(results, "invalid");
            return;
        };
        let request = match NetworkComponentCodec::parse_match_request(fields) {
            Ok(request)
                if request.remote.len()
                    <= store.data().host_limits().network_policy_io_max_bytes =>
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
        let Some(host) = store.data().network_policy_host().cloned() else {
            set_component_string_error(results, "not-found");
            return;
        };
        match host.rules_match_dry_run(request) {
            Ok(result)
                if NetworkComponentCodec::match_result_size(&result)
                    <= store.data().host_limits().network_policy_io_max_bytes =>
            {
                set_component_val_ok(results, NetworkComponentCodec::encode_match_result(result));
            }
            Ok(_) => set_component_string_error(results, "too-large"),
            Err(error) => set_component_string_error(results, &error.message),
        }
    }

    fn rules_apply_or_validate(
        store: wasmtime::StoreContextMut<'_, WasmStoreState>,
        params: &[Val],
        results: &mut [Val],
        apply: bool,
    ) {
        let allowed = if apply {
            store.data().host_grants().can_apply_network_policy_rules()
        } else {
            store
                .data()
                .host_grants()
                .can_validate_network_policy_rules()
        };
        if !allowed {
            set_component_string_error(results, "denied");
            return;
        }
        let [Val::Record(fields)] = params else {
            set_component_string_error(results, "invalid");
            return;
        };
        let request = match NetworkComponentCodec::parse_apply_request(fields) {
            Ok(request)
                if NetworkComponentCodec::apply_request_size(&request)
                    <= store.data().host_limits().network_policy_io_max_bytes =>
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
        let Some(host) = store.data().network_policy_host().cloned() else {
            set_component_string_error(results, "not-found");
            return;
        };
        let Some(owner) = store
            .data()
            .network_policy_owner_instance_id()
            .map(str::to_string)
        else {
            set_component_string_error(results, "not-found");
            return;
        };
        let grants = store
            .data()
            .host_grants()
            .network_policy_rules_apply_grants()
            .to_vec();
        let response = if apply {
            host.rules_apply(&owner, &grants, request)
        } else {
            host.rules_validate(&owner, &grants, &request)
        };
        match response {
            Ok(result)
                if NetworkComponentCodec::apply_result_size(&result)
                    <= store.data().host_limits().network_policy_io_max_bytes =>
            {
                set_component_val_ok(results, NetworkComponentCodec::encode_apply_result(result));
            }
            Ok(_) => set_component_string_error(results, "too-large"),
            Err(error) => set_component_string_error(results, &error.message),
        }
    }

    fn can_access_rules(state: &WasmStoreState) -> bool {
        let grants = state.host_grants();
        grants.can_read_network_policy_rules()
            || grants.can_match_dry_run_network_policy_rules()
            || grants.can_validate_network_policy_rules()
            || grants.can_apply_network_policy_rules()
    }

    fn link_error(error: wasmtime::Error) -> plugin_system::PluginRuntimeError {
        plugin_system::PluginRuntimeError::new(
            "wasm_runtime",
            format!("define network-control component host import failed: {error}"),
        )
    }
}
