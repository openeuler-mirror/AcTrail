//! WIT component control linker and supported-grant admission.

use plugin_system::PluginRuntimeError;
use wasmtime::component::{Func, Linker as ComponentLinker};
use wasmtime::{AsContextMut, Engine};

use crate::engine::{WasmStore, WasmStoreState};
use crate::host::component_read_config;

use super::component_abi;
use super::hostcalls::{
    component_command_execution_context_query, component_command_policy_rules_apply_or_validate,
    component_command_policy_rules_list, component_command_policy_rules_match_dry_run,
    component_command_policy_rules_version_get, component_file_access_current_match_get,
    component_file_policy_rules_apply_or_validate, component_file_policy_rules_list,
    component_file_policy_rules_match_dry_run, component_file_policy_rules_version_get,
    component_query_context,
};
use super::network::NetworkComponentHost;

pub(super) fn find_management_handle_command(
    instance: &wasmtime::component::Instance,
    store: &mut WasmStore,
) -> Option<Func> {
    instance
        .get_export_index(
            store.as_context_mut(),
            None,
            component_abi::MANAGEMENT_COMMAND_EXPORT,
        )
        .and_then(|management| {
            instance.get_export_index(
                store.as_context_mut(),
                Some(&management),
                component_abi::MANAGEMENT_HANDLE_COMMAND_EXPORT,
            )
        })
        .and_then(|export| instance.get_func(store.as_context_mut(), &export))
        .or_else(|| {
            instance.get_func(
                store.as_context_mut(),
                component_abi::MANAGEMENT_HANDLE_COMMAND_FLAT_EXPORT,
            )
        })
}

pub(super) fn component_linker(
    engine: &Engine,
) -> Result<ComponentLinker<WasmStoreState>, PluginRuntimeError> {
    let mut linker = ComponentLinker::new(engine);
    let mut host = linker
        .instance(component_abi::HOST_IMPORT)
        .map_err(|error| {
            PluginRuntimeError::new(
                "wasm_runtime",
                format!("define wasm component host instance failed: {error}"),
            )
        })?;
    host.func_new(
        component_abi::host_import::READ_CONFIG,
        |store, _ty, params, results| {
            component_read_config(store, params, results);
            Ok(())
        },
    )
    .map_err(|error| {
        PluginRuntimeError::new(
            "wasm_runtime",
            format!("define wasm component read-config host import failed: {error}"),
        )
    })?;
    host.func_new(
        component_abi::host_import::QUERY_CONTEXT,
        |store, _ty, params, results| {
            component_query_context(store, params, results);
            Ok(())
        },
    )
    .map_err(|error| {
        PluginRuntimeError::new(
            "wasm_runtime",
            format!("define wasm component query-context host import failed: {error}"),
        )
    })?;
    host.func_new(
        component_abi::host_import::FILE_ACCESS_CURRENT_MATCH_GET,
        |store, _ty, params, results| {
            component_file_access_current_match_get(store, params, results);
            Ok(())
        },
    )
    .map_err(|error| {
        PluginRuntimeError::new(
            "wasm_runtime",
            format!(
                "define wasm component file-access-current-match-get host import failed: {error}"
            ),
        )
    })?;
    host.func_new(
        component_abi::host_import::FILE_POLICY_RULES_VERSION_GET,
        |store, _ty, _params, results| {
            component_file_policy_rules_version_get(store, results);
            Ok(())
        },
    )
    .map_err(|error| {
        PluginRuntimeError::new(
            "wasm_runtime",
            format!(
                "define wasm component file-policy-rules-version-get host import failed: {error}"
            ),
        )
    })?;
    host.func_new(
        component_abi::host_import::FILE_POLICY_RULES_LIST,
        |store, _ty, params, results| {
            component_file_policy_rules_list(store, params, results);
            Ok(())
        },
    )
    .map_err(|error| {
        PluginRuntimeError::new(
            "wasm_runtime",
            format!("define wasm component file-policy-rules-list host import failed: {error}"),
        )
    })?;
    host.func_new(
        component_abi::host_import::FILE_POLICY_RULES_MATCH_DRY_RUN,
        |store, _ty, params, results| {
            component_file_policy_rules_match_dry_run(store, params, results);
            Ok(())
        },
    )
    .map_err(|error| {
        PluginRuntimeError::new(
            "wasm_runtime",
            format!(
                "define wasm component file-policy-rules-match-dry-run host import failed: {error}"
            ),
        )
    })?;
    host.func_new(
        component_abi::host_import::FILE_POLICY_RULES_VALIDATE,
        |store, _ty, params, results| {
            component_file_policy_rules_apply_or_validate(store, params, results, false);
            Ok(())
        },
    )
    .map_err(|error| {
        PluginRuntimeError::new(
            "wasm_runtime",
            format!("define wasm component file-policy-rules-validate host import failed: {error}"),
        )
    })?;
    host.func_new(
        component_abi::host_import::FILE_POLICY_RULES_APPLY,
        |store, _ty, params, results| {
            component_file_policy_rules_apply_or_validate(store, params, results, true);
            Ok(())
        },
    )
    .map_err(|error| {
        PluginRuntimeError::new(
            "wasm_runtime",
            format!("define wasm component file-policy-rules-apply host import failed: {error}"),
        )
    })?;
    host.func_new(
        component_abi::host_import::COMMAND_EXECUTION_CURRENT_CONTEXT_QUERY,
        |store, _ty, params, results| {
            component_command_execution_context_query(store, params, results);
            Ok(())
        },
    )
    .map_err(|error| {
        PluginRuntimeError::new(
            "wasm_runtime",
            format!("define command-execution context host import failed: {error}"),
        )
    })?;
    host.func_new(
        component_abi::host_import::COMMAND_POLICY_RULES_VERSION_GET,
        |store, _ty, _params, results| {
            component_command_policy_rules_version_get(store, results);
            Ok(())
        },
    )
    .map_err(component_command_host_import_error)?;
    host.func_new(
        component_abi::host_import::COMMAND_POLICY_RULES_LIST,
        |store, _ty, params, results| {
            component_command_policy_rules_list(store, params, results);
            Ok(())
        },
    )
    .map_err(component_command_host_import_error)?;
    host.func_new(
        component_abi::host_import::COMMAND_POLICY_RULES_MATCH_DRY_RUN,
        |store, _ty, params, results| {
            component_command_policy_rules_match_dry_run(store, params, results);
            Ok(())
        },
    )
    .map_err(component_command_host_import_error)?;
    host.func_new(
        component_abi::host_import::COMMAND_POLICY_RULES_VALIDATE,
        |store, _ty, params, results| {
            component_command_policy_rules_apply_or_validate(store, params, results, false);
            Ok(())
        },
    )
    .map_err(component_command_host_import_error)?;
    host.func_new(
        component_abi::host_import::COMMAND_POLICY_RULES_APPLY,
        |store, _ty, params, results| {
            component_command_policy_rules_apply_or_validate(store, params, results, true);
            Ok(())
        },
    )
    .map_err(component_command_host_import_error)?;
    drop(host);
    NetworkComponentHost::add_to(&mut linker)?;
    Ok(linker)
}

fn component_command_host_import_error(error: wasmtime::Error) -> PluginRuntimeError {
    PluginRuntimeError::new(
        "wasm_runtime",
        format!("define command-policy component host import failed: {error}"),
    )
}

pub(super) fn is_supported_component_control_grant(grant: &str) -> bool {
    grant == component_abi::grant::CONTEXT_QUERY
        || grant == component_abi::grant::FILE_ACCESS_CURRENT_MATCH_GET
        || grant == component_abi::grant::FILE_POLICY_RULES_READ
        || grant == component_abi::grant::FILE_POLICY_RULES_MATCH_DRY_RUN
        || grant == component_abi::grant::FILE_POLICY_RULES_VALIDATE
        || grant.starts_with(component_abi::grant::FILE_POLICY_RULES_APPLY_PREFIX)
        || grant == component_abi::grant::COMMAND_EXECUTION_CURRENT_CONTEXT_QUERY
        || grant == component_abi::grant::COMMAND_POLICY_RULES_READ
        || grant == component_abi::grant::COMMAND_POLICY_RULES_MATCH_DRY_RUN
        || grant == component_abi::grant::COMMAND_POLICY_RULES_VALIDATE
        || grant.starts_with(component_abi::grant::COMMAND_POLICY_RULES_APPLY_PREFIX)
        || grant == component_abi::grant::NETWORK_ACTION_CURRENT_CONTEXT_QUERY
        || grant == component_abi::grant::NETWORK_POLICY_RULES_READ
        || grant == component_abi::grant::NETWORK_POLICY_RULES_MATCH_DRY_RUN
        || grant == component_abi::grant::NETWORK_POLICY_RULES_VALIDATE
        || grant.starts_with(component_abi::grant::NETWORK_POLICY_RULES_APPLY_PREFIX)
}
