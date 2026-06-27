use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};
use std::time::Instant;

use plugin_system::{
    CONTROL_DECISION_SUMMARY_QUERY, ControlDecider, ControlDecisionBudget, ControlDecisionRequest,
    ControlDecisionResponse, ControlVerdict, DecisionScope, FILE_POLICY_MATCHED_RULE_QUERY,
    PluginHostGrants, PluginHostcallMetricsSource, PluginManifest, PluginRuntimeError,
    PluginRuntimeKind,
};
use wasmtime::Engine;
use wasmtime::component::{Component, Func, Linker as ComponentLinker, Val};

use crate::control::{
    arm_epoch_timeout, call_timeout_error, control_decision_concurrency_limit, disarm_epoch_timeout,
};
use crate::engine::{
    WasmHostcallMetrics, WasmStore, WasmStoreState, fuel_per_call, host_limits, limited_store,
    memory_max_bytes, metered_engine, reset_epoch_deadline_unbounded, reset_fuel,
};
use crate::host::{component_file_policy_write, component_read_config};

mod component_abi {
    pub const CONTROL_DECIDER_EXPORT: &str = "actrail:plugin/control-decider@0.1.0";
    pub const CONTROL_DECIDE_EXPORT: &str = "decide";
    pub const HOST_IMPORT: &str = "actrail:plugin/host@0.1.0";

    pub mod host_import {
        pub const READ_CONFIG: &str = "read-config";
        pub const QUERY_CONTEXT: &str = "query-context";
        pub const FILE_POLICY_READ: &str = "file-policy-read";
        pub const FILE_POLICY_WRITE: &str = "file-policy-write";
    }

    pub mod grant {
        pub const CONTEXT_QUERY: &str = "context-query";
        pub const FILE_POLICY_READ: &str = super::host_import::FILE_POLICY_READ;
        pub const FILE_POLICY_WRITE: &str = super::host_import::FILE_POLICY_WRITE;
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
    ) -> Result<Self, PluginRuntimeError> {
        let host_grant_values = host_grants.to_wire_values();
        let unsupported_grants = host_grant_values.iter().any(|grant| {
            grant != component_abi::grant::CONTEXT_QUERY
                && grant != component_abi::grant::FILE_POLICY_READ
                && grant != component_abi::grant::FILE_POLICY_WRITE
        });
        if unsupported_grants {
            return Err(PluginRuntimeError::new(
                "wasm_runtime",
                "only query-context, file-policy-read, and file-policy-write grants are implemented for WIT component control plugins",
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
            states.push(Mutex::new(WitComponentControlState {
                engine: engine.clone(),
                store,
                decide,
                fuel_per_call,
                deadline_generation: Arc::new(AtomicU64::new(0)),
            }));
        }

        Ok(Self {
            instance_id: instance_id.into(),
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
        component_abi::host_import::FILE_POLICY_READ,
        |store, _ty, params, results| {
            component_file_policy_read(store, params, results);
            Ok(())
        },
    )
    .map_err(|error| {
        PluginRuntimeError::new(
            "wasm_runtime",
            format!("define wasm component file-policy-read host import failed: {error}"),
        )
    })?;
    host.func_new(
        component_abi::host_import::FILE_POLICY_WRITE,
        |store, _ty, params, results| {
            component_file_policy_write(store, params, results);
            Ok(())
        },
    )
    .map_err(|error| {
        PluginRuntimeError::new(
            "wasm_runtime",
            format!("define wasm component file-policy-write host import failed: {error}"),
        )
    })?;
    Ok(linker)
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

fn component_file_policy_read(
    store: wasmtime::StoreContextMut<'_, WasmStoreState>,
    params: &[Val],
    results: &mut [Val],
) {
    if !store.data().host_grants().can_read_file_policy() {
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
        let file_policy_updates = state.store.data_mut().take_file_policy_updates();
        state.store.data_mut().clear_control_context();
        state.store.data_mut().clear_file_policy_context();
        disarm_epoch_timeout(&mut state.store, deadline, &deadline_generation);
        if let Err(error) = result {
            state.store.data_mut().take_file_policy_updates();
            return Err(call_timeout_error(
                &mut state.store,
                "wasm component control decide",
                error,
                budget.timeout_ms,
                started_at,
            ));
        }
        let mut response =
            parse_decision_response(results.into_iter().next().ok_or_else(|| {
                PluginRuntimeError::new("wasm_runtime", "wasm component decide returned no result")
            })?)?;
        response.file_policy_updates = file_policy_updates;
        Ok(response)
    }
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
    fuel_per_call: u64,
    deadline_generation: Arc<AtomicU64>,
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
        file_policy_updates: Vec::new(),
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
