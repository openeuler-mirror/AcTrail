//! Shared synchronous TLS payload launch and event contracts.

mod env;
mod error;
mod event;
mod launch;
mod lookup;
mod plan;
mod runtime;
mod target;

pub use env::{
    ENV_BINARY, ENV_DEPENDENCY_GUARD_DIR, ENV_ENABLED, ENV_EVENT_FD, ENV_EVENT_SOCKET,
    ENV_EVENT_WRITE_BUFFER_BYTES, ENV_EVENTS, ENV_FLOW_CONTROL_ENABLED,
    ENV_FLOW_H2_DATA_PROBE_BYTES, ENV_FLOW_LARGE_TRANSFER_BYTES, ENV_FLOW_MAX_HEADER_BYTES,
    ENV_FLOW_SNIFF_BYTES, ENV_FLOW_UNKNOWN_STREAM_BYTES, ENV_LIBRARY_PATH_PREFIX,
    ENV_LIBRARY_PATH_PREFIX_GLIBC, ENV_LIBRARY_PATH_PREFIX_MUSL, ENV_MAX_PAYLOAD_BYTES,
    ENV_PLAN_BUNDLE, ENV_POINTS, ENV_PROVIDER, ENV_REDACTION, ENV_RULES, ENV_RUNTIME_GLIBC_LIBRARY,
    ENV_RUNTIME_MUSL_LIBRARY, ENV_SYSTEM_LIBRARY_DIRS, ENV_TRACE_ID, EventFilter,
    RUNTIME_GLIBC_LIBRARY_NAME, RUNTIME_MUSL_LIBRARY_NAME, RedactionMode, RuntimeEnvConfig,
    RuntimeFlowControlConfig, runtime_env, runtime_env_for_plan_descriptors, runtime_env_for_plans,
    runtime_plan_descriptor_bundle,
};
pub use error::{SyncError, SyncResult};
pub use event::{
    DecisionEvent, PayloadEvent, SummaryEvent, SyncEvent, decode_event_line, encode_event_line,
    write_event_line,
};
pub use launch::{
    RuntimeLibraryPath, RuntimeLibrarySet, audit_bind_now_env, audit_env_value,
    audit_env_value_for_libraries, audit_libraries_for_plans, launch_command_for_plan,
    launch_command_for_plan_descriptor, preload_env_value, preload_env_value_for_libraries,
    run_with_preload, run_with_preload_libraries, run_with_runtime_libraries,
    runtime_dependency_library_path_env, runtime_dependency_library_path_prefix_env,
    runtime_dependency_library_path_prefix_glibc_env,
    runtime_dependency_library_path_prefix_musl_env, runtime_dependency_report,
    runtime_library_envs, runtime_library_path, runtime_library_set, runtime_musl_library_path,
    validate_native_backend_plan,
};
pub use lookup::{
    PlanLookupRequest, PlanLookupResponse, decode_plan_lookup_request, decode_plan_lookup_response,
    encode_plan_lookup_request, encode_plan_lookup_response, lookup_runtime_plan,
};
pub use plan::{
    RuntimePlanDescriptor, decode_runtime_plan, encode_points, encode_runtime_plan,
    runtime_plan_bundle, runtime_plan_descriptor,
};
pub use runtime::EventClient;
pub use target::{
    LibcFamily, TargetRuntime, resolve_program_path, resolve_target_runtime,
    target_runtime_for_path,
};
