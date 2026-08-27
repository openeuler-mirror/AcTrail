//! WIT component control-decider runtime and instance lifecycle.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};
use std::time::Instant;

use plugin_system::{
    CommandPolicyHost, ControlDecider, ControlDecisionBudget, ControlDecisionRequest,
    ControlDecisionResponse, FilePolicyHost, NetworkPolicyHost, PluginCommandBudget,
    PluginCommandRequest, PluginCommandResponse, PluginHostGrants, PluginHostcallMetricsSource,
    PluginManifest, PluginRuntimeError, PluginRuntimeKind, RuntimePluginConfig,
};
use wasmtime::Engine;
use wasmtime::component::{Component, Func, Val};

use crate::control::{
    arm_epoch_timeout, call_timeout_error, control_decision_concurrency_limit, disarm_epoch_timeout,
};
use crate::engine::{
    WasmHostcallMetrics, WasmStore, fuel_per_call, host_limits, limited_store, memory_max_bytes,
    metered_engine, reset_epoch_deadline_unbounded, reset_fuel,
};

#[path = "component_control/command_codec.rs"]
mod command_codec;
#[path = "component_control/abi.rs"]
mod component_abi;
#[path = "component_control/file_codec.rs"]
mod file_codec;
#[path = "component_control/hostcalls.rs"]
mod hostcalls;
#[path = "component_control/linker.rs"]
mod linker;
#[path = "component_control/network/mod.rs"]
mod network;
#[path = "component_control/runtime_config.rs"]
mod runtime_config;
#[path = "component_control/value.rs"]
mod value;

use linker::{
    component_linker, find_management_handle_command, is_supported_component_control_grant,
};
use runtime_config::RuntimeConfigFunctions;
use value::{
    control_context_snapshot, decision_request_val, parse_decision_response,
    parse_plugin_command_response, plugin_command_request_val, validate_plugin_command_request,
};

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
        command_policy_host: Option<Arc<dyn CommandPolicyHost>>,
        network_policy_host: Option<Arc<dyn NetworkPolicyHost>>,
    ) -> Result<Self, PluginRuntimeError> {
        let instance_id = instance_id.into();
        let host_grant_values = host_grants.to_wire_values();
        let unsupported_grants = host_grant_values
            .iter()
            .any(|grant| !is_supported_component_control_grant(grant));
        if unsupported_grants {
            return Err(PluginRuntimeError::new(
                "wasm_runtime",
                "only context, file-policy, command-policy, and network-policy grants are implemented for WIT component control plugins",
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
            store
                .data_mut()
                .set_command_policy_host(instance_id.clone(), command_policy_host.clone());
            store
                .data_mut()
                .set_network_policy_host(instance_id.clone(), network_policy_host.clone());
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
            let runtime_config = RuntimeConfigFunctions::find(&instance, &mut store)?;
            states.push(Mutex::new(WitComponentControlState {
                engine: engine.clone(),
                store,
                decide,
                handle_command,
                runtime_config,
                fuel_per_call,
                deadline_generation: Arc::new(AtomicU64::new(0)),
            }));
        }

        let decider = Self {
            instance_id,
            plugin_id: manifest.id().to_string(),
            host_grants: host_grant_values,
            hostcall_metrics,
            states,
            next_state: AtomicUsize::new(0),
            instance_concurrency_limit,
        };
        if manifest.plugin_config.runtime_managed {
            let initial_config = plugin_config.ok_or_else(|| {
                PluginRuntimeError::new(
                    "plugin_config",
                    "runtime-managed plugin configuration requires an initial JSON document",
                )
            })?;
            decider.submit_runtime_config(initial_config)?;
        }
        Ok(decider)
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
        state
            .store
            .data_mut()
            .set_command_execution_context(request.command_execution_context.clone());
        state
            .store
            .data_mut()
            .set_network_action_context(request.network_action_context.clone());
        let result = decide.call(&mut state.store, &[input], &mut results);
        state.store.data_mut().clear_control_context();
        state.store.data_mut().clear_file_policy_context();
        state.store.data_mut().clear_command_execution_context();
        state.store.data_mut().clear_network_action_context();
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

    fn runtime_config(&self) -> Result<RuntimePluginConfig, PluginRuntimeError> {
        let mut state = self.lock_state()?;
        let functions = state.runtime_config.clone().ok_or_else(|| {
            PluginRuntimeError::new("plugin_config", "plugin does not export runtime-config")
        })?;
        let config_json = functions.get(&mut state)?;
        Ok(RuntimePluginConfig { config_json })
    }

    fn validate_runtime_config(
        &self,
        config_json: &str,
    ) -> Result<Vec<String>, PluginRuntimeError> {
        let mut state = self.lock_state()?;
        let functions = state.runtime_config.clone().ok_or_else(|| {
            PluginRuntimeError::new("plugin_config", "plugin does not export runtime-config")
        })?;
        functions.validate(&mut state, config_json)
    }

    fn submit_runtime_config(&self, config_json: &str) -> Result<(), PluginRuntimeError> {
        let mut state = self.lock_state()?;
        let functions = state.runtime_config.clone().ok_or_else(|| {
            PluginRuntimeError::new("plugin_config", "plugin does not export runtime-config")
        })?;
        functions.submit(&mut state, config_json)
    }
}

struct WitComponentControlState {
    engine: Engine,
    store: WasmStore,
    decide: Func,
    handle_command: Option<Func>,
    runtime_config: Option<RuntimeConfigFunctions>,
    fuel_per_call: u64,
    deadline_generation: Arc<AtomicU64>,
}
