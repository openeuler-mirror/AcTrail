mod cgroup_v1_binding_event_sink;
mod cgroup_v1_binding_storage;
mod cgroup_v2_binding_storage;
mod managed_scope_storage;
mod resource_profiling_adapter;
mod scope_persistence_adapter;
mod workload_event_sink;
mod workload_target_adapter;
pub(super) use resource_profiling_adapter::{COLLECTOR_NAME, ResourceProfilingAdapter};
