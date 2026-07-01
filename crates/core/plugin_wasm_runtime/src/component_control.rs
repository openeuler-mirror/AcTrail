use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};
use std::time::Instant;

use plugin_system::{
    CONTROL_DECISION_SUMMARY_QUERY, ControlDecider, ControlDecisionBudget, ControlDecisionRequest,
    ControlDecisionResponse, ControlVerdict, DecisionScope, FILE_POLICY_MATCHED_RULE_QUERY,
    FilePolicyApplyMode, FilePolicyApplyPrecondition, FilePolicyApplyRequest,
    FilePolicyApplyResult, FilePolicyApplyStatus, FilePolicyDecision, FilePolicyHost,
    FilePolicyListFilter, FilePolicyListResult, FilePolicyMatchDryRunRequest,
    FilePolicyMatchDryRunResult, FilePolicyOperation, FilePolicyPatchItem, FilePolicyPatchOp,
    FilePolicyRuleDraft, FilePolicyRuleView, PluginCommandBudget, PluginCommandRequest,
    PluginCommandResponse, PluginHostGrants, PluginHostcallMetricsSource, PluginManifest,
    PluginRuntimeError, PluginRuntimeKind,
};
use wasmtime::component::{Component, Func, Linker as ComponentLinker, Val};
use wasmtime::{AsContextMut, Engine};

use crate::control::{
    arm_epoch_timeout, call_timeout_error, control_decision_concurrency_limit, disarm_epoch_timeout,
};
use crate::engine::{
    WasmHostcallMetrics, WasmStore, WasmStoreState, fuel_per_call, host_limits, limited_store,
    memory_max_bytes, metered_engine, reset_epoch_deadline_unbounded, reset_fuel,
};
use crate::host::component_read_config;

mod component_abi {
    pub const CONTROL_DECIDER_EXPORT: &str = "actrail:plugin/control-decider@0.1.0";
    pub const CONTROL_DECIDE_EXPORT: &str = "decide";
    pub const MANAGEMENT_COMMAND_EXPORT: &str = "actrail:plugin/management-command@0.1.0";
    pub const MANAGEMENT_HANDLE_COMMAND_EXPORT: &str = "handle-command";
    pub const MANAGEMENT_HANDLE_COMMAND_FLAT_EXPORT: &str =
        "actrail:plugin/management-command@0.1.0#handle-command";
    pub const HOST_IMPORT: &str = "actrail:plugin/host@0.1.0";

    pub mod host_import {
        pub const READ_CONFIG: &str = "read-config";
        pub const QUERY_CONTEXT: &str = "query-context";
        pub const FILE_ACCESS_CURRENT_MATCH_GET: &str = "file-access-current-match-get";
        pub const FILE_POLICY_RULES_VERSION_GET: &str = "file-policy-rules-version-get";
        pub const FILE_POLICY_RULES_LIST: &str = "file-policy-rules-list";
        pub const FILE_POLICY_RULES_MATCH_DRY_RUN: &str = "file-policy-rules-match-dry-run";
        pub const FILE_POLICY_RULES_VALIDATE: &str = "file-policy-rules-validate";
        pub const FILE_POLICY_RULES_APPLY: &str = "file-policy-rules-apply";
    }

    pub mod grant {
        pub const CONTEXT_QUERY: &str = "context-query";
        pub const FILE_ACCESS_CURRENT_MATCH_GET: &str = "file-access.current-match-get";
        pub const FILE_POLICY_RULES_READ: &str = "file-policy.rules.read";
        pub const FILE_POLICY_RULES_MATCH_DRY_RUN: &str = "file-policy.rules.match-dry-run";
        pub const FILE_POLICY_RULES_VALIDATE: &str = "file-policy.rules.validate";
        pub const FILE_POLICY_RULES_APPLY_PREFIX: &str = "file-policy.rules.apply:";
    }

    pub mod decision_request {
        pub const DECISION_ID: &str = "decision-id";
        pub const TRACE_ID: &str = "trace-id";
        pub const TASK_ID: &str = "task-id";
        pub const SUBJECT: &str = "subject";
        pub const ACTOR_PROCESS_IDENTITY: &str = "actor-process-identity";
        pub const OPERATION: &str = "operation";
        pub const TARGET_SUMMARY: &str = "target-summary";
        pub const CONTEXT_REF: &str = "context-ref";
    }

    pub mod plugin_command_request {
        pub const ARGV: &str = "argv";
    }

    pub mod plugin_command_result {
        pub const EXIT_CODE: &str = "exit-code";
        pub const STDOUT: &str = "stdout";
        pub const STDERR: &str = "stderr";
    }

    pub mod actor_process {
        pub const PID: &str = "pid";
        pub const TASK_ID: &str = "task-id";
        pub const GENERATION: &str = "generation";
        pub const NAMESPACE: &str = "namespace";
    }

    pub mod decision_summary {
        pub const SUBJECT: &str = "subject";
        pub const OPERATION: &str = "operation";
        pub const TARGET_SUMMARY: &str = "target-summary";
        pub const DECISION_ID: &str = "decision-id";
        pub const TRACE_ID: &str = "trace-id";
        pub const ACTOR_PROCESS_IDENTITY: &str = "actor-process-identity";
    }

    pub mod file_policy_view {
        pub const RULE_ID: &str = "rule-id";
        pub const DECISION: &str = "decision";
        pub const OPERATION: &str = "operation";
        pub const PATH: &str = "path";
        pub const PLUGIN_INSTANCE: &str = "plugin-instance";
        pub const TIMEOUT_MS: &str = "timeout-ms";
        pub const CONCURRENCY_LIMIT: &str = "concurrency-limit";
        pub const FALLBACK: &str = "fallback";
    }

    pub mod file_policy_list_filter {
        pub const DECISION: &str = "decision";
        pub const PATH_PREFIX: &str = "path-prefix";
        pub const OPERATION: &str = "operation";
    }

    pub mod file_policy_rule_view {
        pub const RULE_ID: &str = "rule-id";
        pub const OWNER_INSTANCE_ID: &str = "owner-instance-id";
        pub const DECISION: &str = "decision";
        pub const OPERATION: &str = "operation";
        pub const PATH: &str = "path";
        pub const GRAY_TARGET: &str = "gray-target";
        pub const PRIORITY: &str = "priority";
        pub const ENABLED: &str = "enabled";
        pub const UPDATED_SEQUENCE: &str = "updated-sequence";
    }

    pub mod file_policy_list_result {
        pub const RULES: &str = "rules";
        pub const NEXT_CURSOR: &str = "next-cursor";
        pub const SOURCE_REVISION: &str = "source-revision";
    }

    pub mod file_policy_match_dry_run {
        pub const PATH: &str = "path";
        pub const OPERATION: &str = "operation";
        pub const MATCHED: &str = "matched";
        pub const DECISION: &str = "decision";
        pub const RULE_ID: &str = "rule-id";
        pub const CANONICAL_PATH: &str = "canonical-path";
        pub const SOURCE_REVISION: &str = "source-revision";
    }
}

pub(crate) struct WitComponentControlDecider {
    instance_id: String,
    plugin_id: String,
    host_grants: Vec<String>,
    hostcall_metrics: Arc<WasmHostcallMetrics>,
    states: Vec<Mutex<WitComponentControlState>>,
    next_state: AtomicUsize,
    instance_concurrency_limit: u32,
}

impl WitComponentControlDecider {
    pub(crate) fn load(
        instance_id: impl Into<String>,
        manifest: &PluginManifest,
        plugin_config: Option<&str>,
        host_grants: PluginHostGrants,
        file_policy_host: Option<Arc<dyn FilePolicyHost>>,
    ) -> Result<Self, PluginRuntimeError> {
        let instance_id = instance_id.into();
        let host_grant_values = host_grants.to_wire_values();
        let unsupported_grants = host_grant_values
            .iter()
            .any(|grant| !is_supported_component_control_grant(grant));
        if unsupported_grants {
            return Err(PluginRuntimeError::new(
                "wasm_runtime",
                "only query-context and file-access/file-policy rule grants are implemented for WIT component control plugins",
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
        let instance_concurrency_limit = control_decision_concurrency_limit(manifest)?;
        let fuel_per_call = fuel_per_call(manifest);
        let memory_max_bytes = memory_max_bytes(manifest)?;
        let host_limits = host_limits(manifest)?;
        let hostcall_metrics = Arc::new(WasmHostcallMetrics::default());
        let engine = metered_engine()?;
        let component = Component::from_file(&engine, artifact_path).map_err(|error| {
            PluginRuntimeError::new(
                "wasm_runtime",
                format!("load wasm component artifact {artifact_path} failed: {error}"),
            )
        })?;
        let control_export = component
            .get_export_index(None, component_abi::CONTROL_DECIDER_EXPORT)
            .ok_or_else(|| {
                PluginRuntimeError::new(
                    "wasm_runtime",
                    format!(
                        "wasm component missing export {}",
                        component_abi::CONTROL_DECIDER_EXPORT
                    ),
                )
            })?;
        let decide_export = component
            .get_export_index(Some(&control_export), component_abi::CONTROL_DECIDE_EXPORT)
            .ok_or_else(|| {
                PluginRuntimeError::new(
                    "wasm_runtime",
                    format!(
                        "wasm component export {} missing {}",
                        component_abi::CONTROL_DECIDER_EXPORT,
                        component_abi::CONTROL_DECIDE_EXPORT
                    ),
                )
            })?;
        let mut states = Vec::new();
        for _ in 0..instance_concurrency_limit {
            let mut store = limited_store(
                &engine,
                memory_max_bytes,
                host_grants.clone(),
                host_limits.clone(),
                Arc::clone(&hostcall_metrics),
            );
            store
                .data_mut()
                .set_file_policy_host(instance_id.clone(), file_policy_host.clone());
            store.data_mut().set_plugin_config(plugin_config);
            let linker = component_linker(&engine)?;
            reset_fuel(&mut store, fuel_per_call)?;
            let instance = linker
                .instantiate(&mut store, &component)
                .map_err(|error| {
                    PluginRuntimeError::new(
                        "wasm_runtime",
                        format!("instantiate wasm component control plugin failed: {error}"),
                    )
                })?;
            let decide = instance
                .get_func(&mut store, &decide_export)
                .ok_or_else(|| {
                    PluginRuntimeError::new(
                        "wasm_runtime",
                        format!(
                            "wasm component export {}.{} is not a function",
                            component_abi::CONTROL_DECIDER_EXPORT,
                            component_abi::CONTROL_DECIDE_EXPORT
                        ),
                    )
                })?;
            let handle_command = find_management_handle_command(&instance, &mut store);
            states.push(Mutex::new(WitComponentControlState {
                engine: engine.clone(),
                store,
                decide,
                handle_command,
                fuel_per_call,
                deadline_generation: Arc::new(AtomicU64::new(0)),
            }));
        }

        Ok(Self {
            instance_id,
            plugin_id: manifest.id().to_string(),
            host_grants: host_grant_values,
            hostcall_metrics,
            states,
            next_state: AtomicUsize::new(0),
            instance_concurrency_limit,
        })
    }

    fn lock_state(&self) -> Result<MutexGuard<'_, WitComponentControlState>, PluginRuntimeError> {
        if self.states.is_empty() {
            return Err(PluginRuntimeError::new(
                "wasm_runtime",
                "wasm component control decider has no instance state",
            ));
        }
        let start = self.next_state.fetch_add(1, Ordering::Relaxed) % self.states.len();
        for offset in 0..self.states.len() {
            let index = (start + offset) % self.states.len();
            match self.states[index].try_lock() {
                Ok(state) => return Ok(state),
                Err(TryLockError::WouldBlock) => {}
                Err(TryLockError::Poisoned(error)) => {
                    return Err(PluginRuntimeError::new(
                        "wasm_runtime",
                        format!("wasm component state lock poisoned: {error}"),
                    ));
                }
            }
        }
        self.states[start].lock().map_err(|error| {
            PluginRuntimeError::new(
                "wasm_runtime",
                format!("wasm component state lock poisoned: {error}"),
            )
        })
    }
}

fn find_management_handle_command(
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

fn component_linker(
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
    Ok(linker)
}

fn is_supported_component_control_grant(grant: &str) -> bool {
    grant == component_abi::grant::CONTEXT_QUERY
        || grant == component_abi::grant::FILE_ACCESS_CURRENT_MATCH_GET
        || grant == component_abi::grant::FILE_POLICY_RULES_READ
        || grant == component_abi::grant::FILE_POLICY_RULES_MATCH_DRY_RUN
        || grant == component_abi::grant::FILE_POLICY_RULES_VALIDATE
        || grant.starts_with(component_abi::grant::FILE_POLICY_RULES_APPLY_PREFIX)
}

fn component_query_context(
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

fn component_file_access_current_match_get(
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

fn component_file_policy_rules_version_get(
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

fn component_file_policy_rules_list(
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

fn component_file_policy_rules_match_dry_run(
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

fn component_file_policy_rules_apply_or_validate(
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

fn set_component_string_error(results: &mut [Val], message: &str) {
    let Some(result) = results.first_mut() else {
        return;
    };
    *result = Val::Result(Err(Some(Box::new(Val::String(message.to_string())))));
}

fn set_component_val_ok(results: &mut [Val], value: Val) {
    let Some(result) = results.first_mut() else {
        return;
    };
    *result = Val::Result(Ok(Some(Box::new(value))));
}

fn decision_summary_val(context: &crate::engine::ControlContextSnapshot) -> Val {
    Val::Record(vec![
        (
            component_abi::decision_summary::SUBJECT.to_string(),
            Val::Enum(context.subject.clone()),
        ),
        (
            component_abi::decision_summary::OPERATION.to_string(),
            Val::String(context.operation.clone()),
        ),
        (
            component_abi::decision_summary::TARGET_SUMMARY.to_string(),
            Val::String(context.target_summary.clone()),
        ),
        (
            component_abi::decision_summary::DECISION_ID.to_string(),
            Val::String(context.decision_id.clone()),
        ),
        (
            component_abi::decision_summary::TRACE_ID.to_string(),
            Val::String(context.trace_id.clone()),
        ),
        (
            component_abi::decision_summary::ACTOR_PROCESS_IDENTITY.to_string(),
            Val::String(context.actor_process_identity.clone()),
        ),
    ])
}

fn matched_rule_val(context: &plugin_system::FilePolicyReadContext) -> Val {
    let rule = &context.matched_rule;
    Val::Record(vec![
        (
            component_abi::file_policy_view::RULE_ID.to_string(),
            Val::String(rule.rule_id.clone()),
        ),
        (
            component_abi::file_policy_view::DECISION.to_string(),
            Val::String(rule.decision.clone()),
        ),
        (
            component_abi::file_policy_view::OPERATION.to_string(),
            Val::String(rule.operation.clone()),
        ),
        (
            component_abi::file_policy_view::PATH.to_string(),
            Val::String(rule.path.clone()),
        ),
        (
            component_abi::file_policy_view::PLUGIN_INSTANCE.to_string(),
            component_option_string(rule.plugin_instance.clone()),
        ),
        (
            component_abi::file_policy_view::TIMEOUT_MS.to_string(),
            component_option_u64(rule.timeout_ms),
        ),
        (
            component_abi::file_policy_view::CONCURRENCY_LIMIT.to_string(),
            component_option_u32(rule.concurrency_limit),
        ),
        (
            component_abi::file_policy_view::FALLBACK.to_string(),
            component_option_string(rule.fallback.clone()),
        ),
    ])
}

fn component_option_u64(value: Option<u64>) -> Val {
    Val::Option(value.map(|value| Box::new(Val::U64(value))))
}

fn component_option_u32(value: Option<u32>) -> Val {
    Val::Option(value.map(|value| Box::new(Val::U32(value))))
}

fn parse_component_file_policy_list_filter(
    fields: &[(String, Val)],
) -> Result<FilePolicyListFilter, String> {
    let decision =
        match component_field_option(fields, component_abi::file_policy_list_filter::DECISION)? {
            Some(Val::Enum(value)) => Some(parse_component_file_policy_decision(value)?),
            Some(other) => {
                return Err(format!(
                    "field {} option must contain enum, got {other:?}",
                    component_abi::file_policy_list_filter::DECISION
                ));
            }
            None => None,
        };
    let path_prefix =
        component_field_option_string(fields, component_abi::file_policy_list_filter::PATH_PREFIX)?
            .map(str::to_string);
    let operation =
        match component_field_option(fields, component_abi::file_policy_list_filter::OPERATION)? {
            Some(Val::Enum(value)) => Some(parse_component_file_policy_operation(value)?),
            Some(other) => {
                return Err(format!(
                    "field {} option must contain enum, got {other:?}",
                    component_abi::file_policy_list_filter::OPERATION
                ));
            }
            None => None,
        };
    Ok(FilePolicyListFilter {
        decision,
        path_prefix,
        operation,
    })
}

fn parse_component_file_policy_match_dry_run_request(
    fields: &[(String, Val)],
) -> Result<FilePolicyMatchDryRunRequest, String> {
    Ok(FilePolicyMatchDryRunRequest {
        path: component_field_string(fields, component_abi::file_policy_match_dry_run::PATH)?
            .to_string(),
        operation: parse_component_file_policy_operation(component_field_enum(
            fields,
            component_abi::file_policy_match_dry_run::OPERATION,
        )?)?,
    })
}

fn component_file_policy_list_result(result: FilePolicyListResult) -> Val {
    Val::Record(vec![
        (
            component_abi::file_policy_list_result::RULES.to_string(),
            Val::List(
                result
                    .rules
                    .into_iter()
                    .map(component_file_policy_rule_view)
                    .collect(),
            ),
        ),
        (
            component_abi::file_policy_list_result::NEXT_CURSOR.to_string(),
            component_option_string(result.next_cursor),
        ),
        (
            component_abi::file_policy_list_result::SOURCE_REVISION.to_string(),
            Val::U64(result.source_revision),
        ),
    ])
}

fn component_file_policy_rule_view(rule: FilePolicyRuleView) -> Val {
    Val::Record(vec![
        (
            component_abi::file_policy_rule_view::RULE_ID.to_string(),
            Val::String(rule.rule_id),
        ),
        (
            component_abi::file_policy_rule_view::OWNER_INSTANCE_ID.to_string(),
            Val::String(rule.owner_instance_id),
        ),
        (
            component_abi::file_policy_rule_view::DECISION.to_string(),
            component_file_policy_decision(rule.decision),
        ),
        (
            component_abi::file_policy_rule_view::OPERATION.to_string(),
            component_file_policy_operation(rule.operation),
        ),
        (
            component_abi::file_policy_rule_view::PATH.to_string(),
            Val::String(rule.path),
        ),
        (
            component_abi::file_policy_rule_view::GRAY_TARGET.to_string(),
            component_option_u64(rule.gray_target),
        ),
        (
            component_abi::file_policy_rule_view::PRIORITY.to_string(),
            Val::S32(rule.priority),
        ),
        (
            component_abi::file_policy_rule_view::ENABLED.to_string(),
            Val::Bool(rule.enabled),
        ),
        (
            component_abi::file_policy_rule_view::UPDATED_SEQUENCE.to_string(),
            Val::U64(rule.updated_sequence),
        ),
    ])
}

fn component_file_policy_match_dry_run_result(result: FilePolicyMatchDryRunResult) -> Val {
    Val::Record(vec![
        (
            component_abi::file_policy_match_dry_run::MATCHED.to_string(),
            Val::Bool(result.matched),
        ),
        (
            component_abi::file_policy_match_dry_run::DECISION.to_string(),
            component_file_policy_decision(result.decision),
        ),
        (
            component_abi::file_policy_match_dry_run::RULE_ID.to_string(),
            component_option_string(result.rule_id),
        ),
        (
            component_abi::file_policy_match_dry_run::OPERATION.to_string(),
            component_file_policy_operation(result.operation),
        ),
        (
            component_abi::file_policy_match_dry_run::CANONICAL_PATH.to_string(),
            Val::String(result.canonical_path),
        ),
        (
            component_abi::file_policy_match_dry_run::SOURCE_REVISION.to_string(),
            Val::U64(result.source_revision),
        ),
    ])
}

fn component_file_policy_decision(decision: FilePolicyDecision) -> Val {
    Val::Enum(decision.as_str().to_string())
}

fn component_file_policy_operation(operation: FilePolicyOperation) -> Val {
    Val::Enum(operation.as_str().to_string())
}

impl ControlDecider for WitComponentControlDecider {
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

    fn instance_concurrency_limit(&self) -> u32 {
        self.instance_concurrency_limit
    }

    fn decide(
        &self,
        request: ControlDecisionRequest,
        budget: ControlDecisionBudget,
    ) -> Result<ControlDecisionResponse, PluginRuntimeError> {
        let input = decision_request_val(&request);
        let mut state = self.lock_state()?;
        let decide = state.decide.clone();
        let fuel_per_call = state.fuel_per_call;
        reset_fuel(&mut state.store, fuel_per_call)?;
        reset_epoch_deadline_unbounded(&mut state.store);
        let started_at = Instant::now();
        let deadline_generation = state.deadline_generation.clone();
        let deadline = arm_epoch_timeout(
            state.engine.clone(),
            &mut state.store,
            budget.timeout_ms,
            &deadline_generation,
        );
        let mut results = [Val::Result(Ok(None))];
        state
            .store
            .data_mut()
            .set_control_context(control_context_snapshot(&request));
        state
            .store
            .data_mut()
            .set_file_policy_context(request.file_policy_context.clone());
        let result = decide.call(&mut state.store, &[input], &mut results);
        state.store.data_mut().clear_control_context();
        state.store.data_mut().clear_file_policy_context();
        disarm_epoch_timeout(&mut state.store, deadline, &deadline_generation);
        if let Err(error) = result {
            return Err(call_timeout_error(
                &mut state.store,
                "wasm component control decide",
                error,
                budget.timeout_ms,
                started_at,
            ));
        }
        let parsed = parse_decision_response(results.into_iter().next().ok_or_else(|| {
            PluginRuntimeError::new("wasm_runtime", "wasm component decide returned no result")
        })?);
        decide.post_return(&mut state.store).map_err(|error| {
            PluginRuntimeError::new(
                "wasm_runtime",
                format!("wasm component decide post-return failed: {error}"),
            )
        })?;
        parsed
    }

    fn handle_command(
        &self,
        request: PluginCommandRequest,
        budget: PluginCommandBudget,
    ) -> Result<PluginCommandResponse, PluginRuntimeError> {
        let mut state = self.lock_state()?;
        let Some(handle_command) = state.handle_command.clone() else {
            return Err(PluginRuntimeError::new(
                "plugin_command",
                "plugin does not export management-command.handle-command",
            ));
        };
        validate_plugin_command_request(&request, state.store.data().host_limits())?;
        let input = plugin_command_request_val(&request);
        let fuel_per_call = state.fuel_per_call;
        reset_fuel(&mut state.store, fuel_per_call)?;
        reset_epoch_deadline_unbounded(&mut state.store);
        let timeout_ms = budget.timeout_ms.or(Some(
            state.store.data().host_limits().plugin_command_timeout_ms,
        ));
        let started_at = Instant::now();
        let deadline_generation = state.deadline_generation.clone();
        let deadline = arm_epoch_timeout(
            state.engine.clone(),
            &mut state.store,
            timeout_ms,
            &deadline_generation,
        );
        let mut results = [Val::Result(Ok(None))];
        let result = handle_command.call(&mut state.store, &[input], &mut results);
        disarm_epoch_timeout(&mut state.store, deadline, &deadline_generation);
        if let Err(error) = result {
            return Err(call_timeout_error(
                &mut state.store,
                "wasm component plugin command",
                error,
                timeout_ms,
                started_at,
            ));
        }
        let response =
            parse_plugin_command_response(results.into_iter().next().ok_or_else(|| {
                PluginRuntimeError::new("wasm_runtime", "wasm component command returned no result")
            })?);
        handle_command
            .post_return(&mut state.store)
            .map_err(|error| {
                PluginRuntimeError::new(
                    "wasm_runtime",
                    format!("wasm component command post-return failed: {error}"),
                )
            })?;
        let response = response?;
        let output_max_bytes = budget.output_max_bytes.unwrap_or(
            state
                .store
                .data()
                .host_limits()
                .plugin_command_output_max_bytes,
        );
        let output_len = response.stdout.len().saturating_add(response.stderr.len());
        if output_len > output_max_bytes {
            return Err(PluginRuntimeError::new(
                "plugin_command",
                format!("plugin command output exceeded {output_max_bytes} bytes"),
            ));
        }
        Ok(response)
    }
}

fn validate_plugin_command_request(
    request: &PluginCommandRequest,
    limits: &crate::engine::WasmHostLimits,
) -> Result<(), PluginRuntimeError> {
    if request.argv.len() > limits.plugin_command_argv_max_count {
        return Err(PluginRuntimeError::new(
            "plugin_command",
            format!(
                "plugin command argv count exceeded {}",
                limits.plugin_command_argv_max_count
            ),
        ));
    }
    if let Some(arg_len) = request
        .argv
        .iter()
        .map(String::len)
        .find(|arg_len| *arg_len > limits.plugin_command_arg_max_bytes)
    {
        return Err(PluginRuntimeError::new(
            "plugin_command",
            format!(
                "plugin command argument exceeded {} bytes: {arg_len}",
                limits.plugin_command_arg_max_bytes
            ),
        ));
    }
    Ok(())
}

fn control_context_snapshot(
    request: &ControlDecisionRequest,
) -> Option<crate::engine::ControlContextSnapshot> {
    Some(crate::engine::ControlContextSnapshot {
        context_ref: request.context_ref.clone()?,
        decision_id: request.decision_id.clone(),
        trace_id: request.trace_id.clone(),
        subject: request.subject.as_str().to_string(),
        operation: request.operation.clone(),
        target_summary: request.target_summary.clone(),
        actor_process_identity: request.actor_process_identity.summary(),
    })
}

struct WitComponentControlState {
    engine: Engine,
    store: WasmStore,
    decide: Func,
    handle_command: Option<Func>,
    fuel_per_call: u64,
    deadline_generation: Arc<AtomicU64>,
}

fn plugin_command_request_val(request: &PluginCommandRequest) -> Val {
    Val::Record(vec![(
        component_abi::plugin_command_request::ARGV.to_string(),
        Val::List(request.argv.iter().cloned().map(Val::String).collect()),
    )])
}

fn decision_request_val(request: &ControlDecisionRequest) -> Val {
    Val::Record(vec![
        (
            component_abi::decision_request::DECISION_ID.to_string(),
            Val::String(request.decision_id.clone()),
        ),
        (
            component_abi::decision_request::TRACE_ID.to_string(),
            Val::String(request.trace_id.clone()),
        ),
        (
            component_abi::decision_request::TASK_ID.to_string(),
            Val::Option(None),
        ),
        (
            component_abi::decision_request::SUBJECT.to_string(),
            Val::Enum(request.subject.as_str().to_string()),
        ),
        (
            component_abi::decision_request::ACTOR_PROCESS_IDENTITY.to_string(),
            actor_process_identity_val(&request.actor_process_identity),
        ),
        (
            component_abi::decision_request::OPERATION.to_string(),
            Val::String(request.operation.clone()),
        ),
        (
            component_abi::decision_request::TARGET_SUMMARY.to_string(),
            Val::String(request.target_summary.clone()),
        ),
        (
            component_abi::decision_request::CONTEXT_REF.to_string(),
            component_option_string(request.context_ref.clone()),
        ),
    ])
}

fn actor_process_identity_val(actor: &plugin_system::ControlActorProcessIdentity) -> Val {
    Val::Record(vec![
        (
            component_abi::actor_process::PID.to_string(),
            Val::U32(actor.pid),
        ),
        (
            component_abi::actor_process::TASK_ID.to_string(),
            Val::Option(actor.task_id.map(|task_id| Box::new(Val::U32(task_id)))),
        ),
        (
            component_abi::actor_process::GENERATION.to_string(),
            Val::U64(actor.generation),
        ),
        (
            component_abi::actor_process::NAMESPACE.to_string(),
            component_option_string(actor.namespace.clone()),
        ),
    ])
}

fn component_option_string(value: Option<String>) -> Val {
    Val::Option(value.map(|value| Box::new(Val::String(value))))
}

fn parse_component_file_policy_apply_request(
    fields: &[(String, Val)],
) -> Result<FilePolicyApplyRequest, String> {
    let base_revision = component_field_u64(fields, "base-revision")?;
    let mutation_id = component_field_string(fields, "mutation-id")?.to_string();
    let reason = component_field_option_string(fields, "reason")?.map(str::to_string);
    let correlation_id =
        component_field_option_string(fields, "correlation-id")?.map(str::to_string);
    let apply_mode = match component_field_enum(fields, "apply-mode")? {
        "partial" => FilePolicyApplyMode::Partial,
        "aon" => FilePolicyApplyMode::Aon,
        other => return Err(format!("unsupported apply-mode {other}")),
    };
    let items = component_field_list(fields, "items")?
        .iter()
        .map(parse_component_file_policy_patch_item)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(FilePolicyApplyRequest {
        items,
        precondition: FilePolicyApplyPrecondition {
            base_revision,
            mutation_id,
            reason,
            correlation_id,
            apply_mode,
        },
    })
}

fn parse_component_file_policy_patch_item(value: &Val) -> Result<FilePolicyPatchItem, String> {
    let Val::Record(fields) = value else {
        return Err("file-policy patch item must be a record".to_string());
    };
    let op = match component_field_enum(fields, "op")? {
        "upsert" => FilePolicyPatchOp::Upsert,
        "delete" => FilePolicyPatchOp::Delete,
        "enable" => FilePolicyPatchOp::Enable,
        "disable" => FilePolicyPatchOp::Disable,
        other => return Err(format!("unsupported patch op {other}")),
    };
    let rule_id = component_field_option_string(fields, "rule-id")?.map(str::to_string);
    let rule = match component_field_option(fields, "rule")? {
        Some(Val::Record(rule_fields)) => {
            Some(parse_component_file_policy_rule_draft(rule_fields)?)
        }
        Some(_) => return Err("file-policy patch rule must be a record".to_string()),
        None => None,
    };
    Ok(FilePolicyPatchItem { op, rule_id, rule })
}

fn parse_component_file_policy_rule_draft(
    fields: &[(String, Val)],
) -> Result<FilePolicyRuleDraft, String> {
    let rule_id = component_field_option_string(fields, "rule-id")?.map(str::to_string);
    let decision = parse_component_file_policy_decision(component_field_enum(fields, "decision")?)?;
    let operation =
        parse_component_file_policy_operation(component_field_enum(fields, "operation")?)?;
    Ok(FilePolicyRuleDraft {
        rule_id,
        decision,
        operation,
        path: component_field_string(fields, "path")?.to_string(),
        gray_target: component_field_option_u64(fields, "gray-target")?,
        priority: component_field_s32(fields, "priority")?,
    })
}

fn component_file_policy_apply_result(result: FilePolicyApplyResult) -> Val {
    Val::Record(vec![
        (
            "status".to_string(),
            Val::Enum(
                match result.status {
                    FilePolicyApplyStatus::Accepted => "accepted",
                    FilePolicyApplyStatus::Rejected => "rejected",
                }
                .to_string(),
            ),
        ),
        ("new-revision".to_string(), Val::U64(result.new_revision)),
        ("applied-count".to_string(), Val::U32(result.applied_count)),
        (
            "rejected-count".to_string(),
            Val::U32(result.rejected_count),
        ),
        (
            "errors".to_string(),
            Val::List(
                result
                    .errors
                    .into_iter()
                    .map(|error| {
                        Val::Record(vec![
                            ("item-index".to_string(), Val::U32(error.item_index)),
                            ("code".to_string(), Val::String(error.code)),
                            ("message".to_string(), Val::String(error.message)),
                        ])
                    })
                    .collect(),
            ),
        ),
    ])
}

fn parse_component_file_policy_decision(value: &str) -> Result<FilePolicyDecision, String> {
    FilePolicyDecision::from_wire(value)
}

fn parse_component_file_policy_operation(value: &str) -> Result<FilePolicyOperation, String> {
    FilePolicyOperation::from_wire(value)
}

fn parse_component_option_string_val(value: &Val) -> Result<Option<String>, String> {
    match value {
        Val::Option(Some(value)) => match value.as_ref() {
            Val::String(value) => Ok(Some(value.clone())),
            other => Err(format!("option must contain string, got {other:?}")),
        },
        Val::Option(None) => Ok(None),
        other => Err(format!("value must be option<string>, got {other:?}")),
    }
}

fn component_field<'a>(fields: &'a [(String, Val)], name: &str) -> Result<&'a Val, String> {
    fields
        .iter()
        .find(|(field, _)| field == name)
        .map(|(_, value)| value)
        .ok_or_else(|| format!("missing field {name}"))
}

fn component_field_enum<'a>(fields: &'a [(String, Val)], name: &str) -> Result<&'a str, String> {
    match component_field(fields, name)? {
        Val::Enum(value) => Ok(value),
        other => Err(format!("field {name} must be enum, got {other:?}")),
    }
}

fn component_field_string<'a>(fields: &'a [(String, Val)], name: &str) -> Result<&'a str, String> {
    match component_field(fields, name)? {
        Val::String(value) => Ok(value),
        other => Err(format!("field {name} must be string, got {other:?}")),
    }
}

fn component_field_u64(fields: &[(String, Val)], name: &str) -> Result<u64, String> {
    match component_field(fields, name)? {
        Val::U64(value) => Ok(*value),
        other => Err(format!("field {name} must be u64, got {other:?}")),
    }
}

fn component_field_s32(fields: &[(String, Val)], name: &str) -> Result<i32, String> {
    match component_field(fields, name)? {
        Val::S32(value) => Ok(*value),
        other => Err(format!("field {name} must be s32, got {other:?}")),
    }
}

fn component_field_list<'a>(fields: &'a [(String, Val)], name: &str) -> Result<&'a [Val], String> {
    match component_field(fields, name)? {
        Val::List(values) => Ok(values),
        other => Err(format!("field {name} must be list, got {other:?}")),
    }
}

fn component_field_option<'a>(
    fields: &'a [(String, Val)],
    name: &str,
) -> Result<Option<&'a Val>, String> {
    match component_field(fields, name)? {
        Val::Option(Some(value)) => Ok(Some(value)),
        Val::Option(None) => Ok(None),
        other => Err(format!("field {name} must be option, got {other:?}")),
    }
}

fn component_field_option_string<'a>(
    fields: &'a [(String, Val)],
    name: &str,
) -> Result<Option<&'a str>, String> {
    match component_field_option(fields, name)? {
        Some(Val::String(value)) => Ok(Some(value)),
        Some(other) => Err(format!(
            "field {name} option must contain string, got {other:?}"
        )),
        None => Ok(None),
    }
}

fn component_field_option_u64(fields: &[(String, Val)], name: &str) -> Result<Option<u64>, String> {
    match component_field_option(fields, name)? {
        Some(Val::U64(value)) => Ok(Some(*value)),
        Some(other) => Err(format!(
            "field {name} option must contain u64, got {other:?}"
        )),
        None => Ok(None),
    }
}

fn parse_decision_response(value: Val) -> Result<ControlDecisionResponse, PluginRuntimeError> {
    let response = match value {
        Val::Result(Ok(Some(ok))) => *ok,
        Val::Result(Ok(None)) => {
            return Err(PluginRuntimeError::new(
                "wasm_runtime",
                "wasm component decide returned ok without decision-response",
            ));
        }
        Val::Result(Err(Some(error))) => {
            let message = match *error {
                Val::String(message) => message,
                other => format!("{other:?}"),
            };
            return Err(PluginRuntimeError::new(
                "wasm_runtime",
                format!("wasm component decide returned error: {message}"),
            ));
        }
        Val::Result(Err(None)) => {
            return Err(PluginRuntimeError::new(
                "wasm_runtime",
                "wasm component decide returned error without message",
            ));
        }
        other => {
            return Err(PluginRuntimeError::new(
                "wasm_runtime",
                format!("wasm component decide returned invalid result {other:?}"),
            ));
        }
    };
    let fields = match response {
        Val::Record(fields) => fields,
        other => {
            return Err(PluginRuntimeError::new(
                "wasm_runtime",
                format!("wasm component decide returned invalid response {other:?}"),
            ));
        }
    };
    let verdict = match decision_field_enum(&fields, "verdict")?.as_str() {
        "allow" => ControlVerdict::Allow,
        "deny" => ControlVerdict::Deny,
        other => {
            return Err(PluginRuntimeError::new(
                "wasm_runtime",
                format!("wasm component decide returned unsupported verdict {other}"),
            ));
        }
    };
    let scope = match decision_field_enum(&fields, "scope")?.as_str() {
        "once" => DecisionScope::Once,
        "reusable" => DecisionScope::Reusable,
        other => {
            return Err(PluginRuntimeError::new(
                "wasm_runtime",
                format!("wasm component decide returned unsupported scope {other}"),
            ));
        }
    };
    let reason_code = decision_field_option_string(&fields, "reason-code")?;
    let reason_message = decision_field_option_string(&fields, "reason-message")?;
    Ok(ControlDecisionResponse {
        verdict,
        scope,
        reason: reason_message.or(reason_code),
    })
}

fn parse_plugin_command_response(value: Val) -> Result<PluginCommandResponse, PluginRuntimeError> {
    let response = match value {
        Val::Result(Ok(Some(ok))) => *ok,
        Val::Result(Ok(None)) => {
            return Err(PluginRuntimeError::new(
                "wasm_runtime",
                "wasm component command returned ok without plugin-command-result",
            ));
        }
        Val::Result(Err(Some(error))) => {
            let message = match *error {
                Val::String(message) => message,
                other => format!("{other:?}"),
            };
            return Err(PluginRuntimeError::new(
                "plugin_command",
                format!("wasm component command returned error: {message}"),
            ));
        }
        Val::Result(Err(None)) => {
            return Err(PluginRuntimeError::new(
                "plugin_command",
                "wasm component command returned error without message",
            ));
        }
        other => {
            return Err(PluginRuntimeError::new(
                "wasm_runtime",
                format!("wasm component command returned invalid result {other:?}"),
            ));
        }
    };
    let fields = match response {
        Val::Record(fields) => fields,
        other => {
            return Err(PluginRuntimeError::new(
                "wasm_runtime",
                format!("wasm component command returned invalid response {other:?}"),
            ));
        }
    };
    Ok(PluginCommandResponse {
        exit_code: component_field_s32(&fields, component_abi::plugin_command_result::EXIT_CODE)
            .map_err(|error| PluginRuntimeError::new("wasm_runtime", error))?,
        stdout: component_field_string(&fields, component_abi::plugin_command_result::STDOUT)
            .map(str::to_string)
            .map_err(|error| PluginRuntimeError::new("wasm_runtime", error))?,
        stderr: component_field_string(&fields, component_abi::plugin_command_result::STDERR)
            .map(str::to_string)
            .map_err(|error| PluginRuntimeError::new("wasm_runtime", error))?,
    })
}

fn decision_field_enum(fields: &[(String, Val)], name: &str) -> Result<String, PluginRuntimeError> {
    match fields.iter().find(|(field, _)| field == name) {
        Some((_, Val::Enum(value))) => Ok(value.clone()),
        Some((_, other)) => Err(PluginRuntimeError::new(
            "wasm_runtime",
            format!("wasm component decision field {name} has invalid value {other:?}"),
        )),
        None => Err(PluginRuntimeError::new(
            "wasm_runtime",
            format!("wasm component decision missing field {name}"),
        )),
    }
}

fn decision_field_option_string(
    fields: &[(String, Val)],
    name: &str,
) -> Result<Option<String>, PluginRuntimeError> {
    match fields.iter().find(|(field, _)| field == name) {
        Some((_, Val::Option(Some(value)))) => match value.as_ref() {
            Val::String(value) => Ok(Some(value.clone())),
            other => Err(PluginRuntimeError::new(
                "wasm_runtime",
                format!("wasm component decision field {name} has invalid value {other:?}"),
            )),
        },
        Some((_, Val::Option(None))) => Ok(None),
        Some((_, other)) => Err(PluginRuntimeError::new(
            "wasm_runtime",
            format!("wasm component decision field {name} has invalid value {other:?}"),
        )),
        None => Err(PluginRuntimeError::new(
            "wasm_runtime",
            format!("wasm component decision missing field {name}"),
        )),
    }
}
