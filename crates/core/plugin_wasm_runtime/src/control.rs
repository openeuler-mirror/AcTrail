use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};
use std::time::{Duration, Instant};

use plugin_system::{
    ControlDecider, ControlDecisionBudget, ControlDecisionRequest, ControlDecisionResponse,
    ControlVerdict, DecisionScope, FilePolicyHost, PluginCommandBudget, PluginCommandRequest,
    PluginCommandResponse, PluginHostGrants, PluginHostcallMetricsSource, PluginManifest,
    PluginRuntimeError, PluginRuntimeKind, PluginWasmAbi, RuntimePluginConfig,
};
use serde_json::{Map, Value, json};
use wasmtime::{Engine, Memory, Module, TypedFunc};

use crate::component_control::WitComponentControlDecider;
use crate::engine::{
    ControlContextSnapshot, WasmHostcallMetrics, WasmStore, call_error, fuel_per_call, host_limits,
    limited_store, memory_max_bytes, metered_engine, reset_epoch_deadline_unbounded, reset_fuel,
};
use crate::host::host_linker;
use crate::memory::write_guest_bytes;

mod control_envelope {
    pub const VERSION: u8 = 1;

    pub mod key {
        pub const VERSION: &str = "v";
        pub const DECISION_ID: &str = "id";
        pub const TRACE_ID: &str = "tr";
        pub const SUBJECT: &str = "s";
        pub const ACTOR: &str = "a";
        pub const OPERATION: &str = "op";
        pub const TARGET: &str = "t";
        pub const CONTEXT: &str = "ctx";
    }

    pub mod actor_key {
        pub const PID: &str = "pid";
        pub const TASK_ID: &str = "tid";
        pub const GENERATION: &str = "gen";
        pub const NAMESPACE: &str = "ns";
    }
}

pub fn build_wasm_control_decider(
    instance_id: impl Into<String>,
    manifest: &PluginManifest,
    plugin_config: Option<&str>,
    host_grants: PluginHostGrants,
    file_policy_host: Option<Arc<dyn FilePolicyHost>>,
) -> Result<WasmControlDecider, PluginRuntimeError> {
    let instance_id = instance_id.into();
    let wasm = manifest.selected_wasm().ok_or_else(|| {
        PluginRuntimeError::new(
            "wasm_runtime",
            "wasm plugin manifest missing [runtime.wasm]",
        )
    })?;
    match wasm.abi {
        PluginWasmAbi::LegacyModule => {
            let decider = LegacyWasmControlDecider::load(
                instance_id,
                manifest,
                plugin_config.unwrap_or_default(),
                host_grants,
                file_policy_host,
            )?;
            Ok(WasmControlDecider::new(Box::new(decider)))
        }
        PluginWasmAbi::WitComponent => {
            let decider = WitComponentControlDecider::load(
                instance_id,
                manifest,
                plugin_config,
                host_grants,
                file_policy_host,
            )?;
            Ok(WasmControlDecider::new(Box::new(decider)))
        }
    }
}

pub struct WasmControlDecider {
    inner: Box<dyn ControlDecider>,
}

impl WasmControlDecider {
    fn new(inner: Box<dyn ControlDecider>) -> Self {
        Self { inner }
    }
}

impl ControlDecider for WasmControlDecider {
    fn instance_id(&self) -> &str {
        self.inner.instance_id()
    }

    fn plugin_id(&self) -> &str {
        self.inner.plugin_id()
    }

    fn runtime_kind(&self) -> PluginRuntimeKind {
        self.inner.runtime_kind()
    }

    fn host_grants(&self) -> Vec<String> {
        self.inner.host_grants()
    }

    fn hostcall_metrics_source(&self) -> Option<Arc<dyn PluginHostcallMetricsSource>> {
        self.inner.hostcall_metrics_source()
    }

    fn instance_concurrency_limit(&self) -> u32 {
        self.inner.instance_concurrency_limit()
    }

    fn decide(
        &self,
        request: ControlDecisionRequest,
        budget: ControlDecisionBudget,
    ) -> Result<ControlDecisionResponse, PluginRuntimeError> {
        self.inner.decide(request, budget)
    }

    fn handle_command(
        &self,
        request: PluginCommandRequest,
        budget: PluginCommandBudget,
    ) -> Result<PluginCommandResponse, PluginRuntimeError> {
        self.inner.handle_command(request, budget)
    }

    fn runtime_config(&self) -> Result<RuntimePluginConfig, PluginRuntimeError> {
        self.inner.runtime_config()
    }

    fn validate_runtime_config(
        &self,
        config_json: &str,
    ) -> Result<Vec<String>, PluginRuntimeError> {
        self.inner.validate_runtime_config(config_json)
    }

    fn submit_runtime_config(&self, config_json: &str) -> Result<(), PluginRuntimeError> {
        self.inner.submit_runtime_config(config_json)
    }
}

struct LegacyWasmControlDecider {
    instance_id: String,
    plugin_id: String,
    host_grants: Vec<String>,
    hostcall_metrics: Arc<WasmHostcallMetrics>,
    states: Vec<Mutex<WasmControlState>>,
    next_state: AtomicUsize,
    instance_concurrency_limit: u32,
}

impl LegacyWasmControlDecider {
    fn load(
        instance_id: String,
        manifest: &PluginManifest,
        plugin_config: &str,
        host_grants: PluginHostGrants,
        file_policy_host: Option<Arc<dyn FilePolicyHost>>,
    ) -> Result<Self, PluginRuntimeError> {
        let artifact_path = manifest
            .selected_wasm()
            .and_then(|wasm| wasm.artifact_path.as_deref())
            .ok_or_else(|| {
                PluginRuntimeError::new(
                    "wasm_runtime",
                    "wasm plugin manifest missing [runtime.wasm]",
                )
            })?;
        let host_grant_values = host_grants.to_wire_values();
        let instance_concurrency_limit = control_decision_concurrency_limit(manifest)?;
        let fuel_per_call = fuel_per_call(manifest);
        let memory_max_bytes = memory_max_bytes(manifest)?;
        let host_limits = host_limits(manifest)?;
        let hostcall_metrics = Arc::new(WasmHostcallMetrics::default());
        let engine = metered_engine()?;
        let module = Module::from_file(&engine, artifact_path).map_err(|error| {
            PluginRuntimeError::new(
                "wasm_runtime",
                format!("load wasm artifact {artifact_path} failed: {error}"),
            )
        })?;
        let mut states = Vec::new();
        for _ in 0..instance_concurrency_limit {
            states.push(Mutex::new(instantiate_control_state(
                engine.clone(),
                &module,
                memory_max_bytes,
                fuel_per_call,
                plugin_config,
                host_grants.clone(),
                host_limits.clone(),
                Arc::clone(&hostcall_metrics),
                instance_id.clone(),
                file_policy_host.clone(),
            )?));
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

    fn lock_state(&self) -> Result<MutexGuard<'_, WasmControlState>, PluginRuntimeError> {
        if self.states.is_empty() {
            return Err(PluginRuntimeError::new(
                "wasm_runtime",
                "wasm control decider has no instance state",
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
                        format!("wasm state lock poisoned: {error}"),
                    ));
                }
            }
        }
        self.states[start].lock().map_err(|error| {
            PluginRuntimeError::new("wasm_runtime", format!("wasm state lock poisoned: {error}"))
        })
    }
}

fn instantiate_control_state(
    engine: Engine,
    module: &Module,
    memory_max_bytes: usize,
    fuel_per_call: u64,
    plugin_config: &str,
    host_grants: PluginHostGrants,
    host_limits: crate::engine::WasmHostLimits,
    hostcall_metrics: Arc<WasmHostcallMetrics>,
    instance_id: String,
    file_policy_host: Option<Arc<dyn FilePolicyHost>>,
) -> Result<WasmControlState, PluginRuntimeError> {
    let mut store = limited_store(
        &engine,
        memory_max_bytes,
        host_grants,
        host_limits,
        hostcall_metrics,
    );
    store
        .data_mut()
        .set_file_policy_host(instance_id, file_policy_host);
    let linker = host_linker(&engine)?;
    let instance = linker.instantiate(&mut store, module).map_err(|error| {
        PluginRuntimeError::new(
            "wasm_runtime",
            format!("instantiate wasm plugin failed: {error}"),
        )
    })?;
    let memory = instance.get_memory(&mut store, "memory").ok_or_else(|| {
        PluginRuntimeError::new("wasm_runtime", "wasm plugin missing memory export")
    })?;
    let alloc = instance
        .get_typed_func::<i32, i32>(&mut store, "actrail_alloc")
        .map_err(|error| {
            PluginRuntimeError::new(
                "wasm_runtime",
                format!("wasm plugin missing actrail_alloc: {error}"),
            )
        })?;
    let decide = instance
        .get_typed_func::<(i32, i32), i64>(&mut store, "actrail_control_decide")
        .map_err(|error| {
            PluginRuntimeError::new(
                "wasm_runtime",
                format!("wasm plugin missing actrail_control_decide: {error}"),
            )
        })?;
    let init = instance
        .get_typed_func::<(i32, i32), i32>(&mut store, "actrail_plugin_init")
        .ok();

    if let Some(init) = init {
        reset_fuel(&mut store, fuel_per_call)?;
        let (ptr, len) =
            write_guest_bytes(&mut store, memory, alloc.clone(), plugin_config.as_bytes())?;
        let status = init
            .call(&mut store, (ptr, len))
            .map_err(|error| call_error(&mut store, "wasm plugin init", error))?;
        if status != 0 {
            return Err(PluginRuntimeError::new(
                "wasm_runtime",
                format!("wasm plugin init returned {status}"),
            ));
        }
    }

    Ok(WasmControlState {
        engine,
        store,
        memory,
        alloc,
        decide,
        fuel_per_call,
        deadline_generation: Arc::new(AtomicU64::new(0)),
    })
}

pub(crate) fn control_decision_concurrency_limit(
    manifest: &PluginManifest,
) -> Result<u32, PluginRuntimeError> {
    let limit = manifest.control_decision_concurrency_limit().unwrap_or(1);
    if limit == 0 {
        return Err(PluginRuntimeError::new(
            "plugin_manifest",
            "role.control-decider.resources.concurrency_limit must be greater than zero",
        ));
    }
    Ok(limit)
}

impl ControlDecider for LegacyWasmControlDecider {
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
        let envelope = control_envelope(&request)?;
        let mut state = self.lock_state()?;
        let memory = state.memory;
        let alloc = state.alloc.clone();
        let fuel_per_call = state.fuel_per_call;
        reset_fuel(&mut state.store, fuel_per_call)?;
        reset_epoch_deadline_unbounded(&mut state.store);
        let (ptr, len) = write_guest_bytes(&mut state.store, memory, alloc, envelope.as_bytes())?;
        state
            .store
            .data_mut()
            .set_control_context(control_context_snapshot(&request));
        state
            .store
            .data_mut()
            .set_file_policy_context(request.file_policy_context.clone());
        let decide = state.decide.clone();
        let started_at = Instant::now();
        let deadline_generation = state.deadline_generation.clone();
        let deadline = arm_epoch_timeout(
            state.engine.clone(),
            &mut state.store,
            budget.timeout_ms,
            &deadline_generation,
        );
        let result = decide.call(&mut state.store, (ptr, len));
        state.store.data_mut().clear_control_context();
        state.store.data_mut().clear_file_policy_context();
        disarm_epoch_timeout(&mut state.store, deadline, &deadline_generation);
        let code = result.map_err(|error| {
            call_timeout_error(
                &mut state.store,
                "wasm control decide",
                error,
                budget.timeout_ms,
                started_at,
            )
        })?;
        decision_from_code(code)
    }
}

struct WasmControlState {
    engine: Engine,
    store: WasmStore,
    memory: Memory,
    alloc: TypedFunc<i32, i32>,
    decide: TypedFunc<(i32, i32), i64>,
    fuel_per_call: u64,
    deadline_generation: Arc<AtomicU64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct EpochDeadline {
    generation: u64,
}

pub(crate) fn arm_epoch_timeout(
    engine: Engine,
    store: &mut WasmStore,
    timeout_ms: Option<u64>,
    deadline_generation: &Arc<AtomicU64>,
) -> Option<EpochDeadline> {
    let Some(timeout_ms) = timeout_ms else {
        reset_epoch_deadline_unbounded(store);
        return None;
    };
    let generation = deadline_generation.fetch_add(1, Ordering::SeqCst) + 1;
    store.set_epoch_deadline(1);
    let deadline_generation = Arc::clone(deadline_generation);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(timeout_ms));
        if deadline_generation.load(Ordering::SeqCst) == generation {
            engine.increment_epoch();
        }
    });
    Some(EpochDeadline { generation })
}

pub(crate) fn disarm_epoch_timeout(
    store: &mut WasmStore,
    deadline: Option<EpochDeadline>,
    deadline_generation: &AtomicU64,
) {
    if let Some(deadline) = deadline {
        deadline_generation
            .compare_exchange(
                deadline.generation,
                deadline.generation + 1,
                Ordering::SeqCst,
                Ordering::SeqCst,
            )
            .ok();
    }
    reset_epoch_deadline_unbounded(store);
}

pub(crate) fn call_timeout_error(
    store: &mut WasmStore,
    operation: &str,
    error: impl std::fmt::Display,
    timeout_ms: Option<u64>,
    started_at: Instant,
) -> PluginRuntimeError {
    if store.get_fuel().map(|fuel| fuel == 0).unwrap_or(false) {
        return call_error(store, operation, error);
    }
    if let Some(timeout_ms) = timeout_ms {
        if started_at.elapsed() >= Duration::from_millis(timeout_ms) {
            return PluginRuntimeError::new(
                "wasm_runtime",
                format!("wasm timeout after {timeout_ms}ms during {operation}: {error}"),
            );
        }
    }
    call_error(store, operation, error)
}

fn decision_from_code(code: i64) -> Result<ControlDecisionResponse, PluginRuntimeError> {
    let (verdict, scope) = match code {
        1 => (ControlVerdict::Allow, DecisionScope::Once),
        2 => (ControlVerdict::Allow, DecisionScope::Reusable),
        -1 => (ControlVerdict::Deny, DecisionScope::Once),
        -2 => (ControlVerdict::Deny, DecisionScope::Reusable),
        _ => {
            return Err(PluginRuntimeError::new(
                "wasm_runtime",
                format!("wasm control decide returned unsupported code {code}"),
            ));
        }
    };
    Ok(ControlDecisionResponse {
        verdict,
        scope,
        reason: None,
    })
}

fn control_context_snapshot(request: &ControlDecisionRequest) -> Option<ControlContextSnapshot> {
    Some(ControlContextSnapshot {
        context_ref: request.context_ref.clone()?,
        decision_id: request.decision_id.clone(),
        trace_id: request.trace_id.clone(),
        subject: request.subject.as_str().to_string(),
        operation: request.operation.clone(),
        target_summary: request.target_summary.clone(),
        actor_process_identity: request.actor_process_identity.summary(),
    })
}

fn control_envelope(request: &ControlDecisionRequest) -> Result<String, PluginRuntimeError> {
    let actor = &request.actor_process_identity;
    let mut actor_fields = Map::new();
    actor_fields.insert(
        control_envelope::actor_key::PID.to_string(),
        json!(actor.pid),
    );
    actor_fields.insert(
        control_envelope::actor_key::TASK_ID.to_string(),
        json!(actor.task_id),
    );
    actor_fields.insert(
        control_envelope::actor_key::GENERATION.to_string(),
        json!(actor.generation),
    );
    actor_fields.insert(
        control_envelope::actor_key::NAMESPACE.to_string(),
        json!(actor.namespace),
    );

    let mut envelope = Map::new();
    envelope.insert(
        control_envelope::key::VERSION.to_string(),
        json!(control_envelope::VERSION),
    );
    envelope.insert(
        control_envelope::key::DECISION_ID.to_string(),
        json!(request.decision_id),
    );
    envelope.insert(
        control_envelope::key::TRACE_ID.to_string(),
        json!(request.trace_id),
    );
    envelope.insert(
        control_envelope::key::SUBJECT.to_string(),
        json!(request.subject.code()),
    );
    envelope.insert(
        control_envelope::key::ACTOR.to_string(),
        Value::Object(actor_fields),
    );
    envelope.insert(
        control_envelope::key::OPERATION.to_string(),
        json!(request.operation),
    );
    envelope.insert(
        control_envelope::key::TARGET.to_string(),
        json!(request.target_summary),
    );
    envelope.insert(
        control_envelope::key::CONTEXT.to_string(),
        json!(request.context_ref),
    );

    serde_json::to_string(&Value::Object(envelope)).map_err(|error| {
        PluginRuntimeError::new(
            "wasm_runtime",
            format!("encode control envelope failed: {error}"),
        )
    })
}
