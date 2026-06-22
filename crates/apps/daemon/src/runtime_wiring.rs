//! Dependency wiring from contracts to runtime packages.

pub struct DaemonRuntimeWiring<A> {
    pub trace_runtime: trace_runtime::TraceRuntime,
    pub attach_service: A,
    pub active_trace_max: u32,
    pub available_collectors: Vec<String>,
    pub loaded_policy_plugins: Vec<String>,
    pub storage_ready: bool,
}
