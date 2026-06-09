//! Shared synchronous TLS payload launch and event contracts.

mod env;
mod error;
mod event;
mod launch;
mod lookup;
mod plan;
mod runtime;

pub use env::{
    ENV_BINARY, ENV_ENABLED, ENV_EVENT_SOCKET, ENV_EVENTS, ENV_MAX_PAYLOAD_BYTES, ENV_PLAN_BUNDLE,
    ENV_POINTS, ENV_PROVIDER, ENV_REDACTION, ENV_RULES, ENV_TRACE_ID, EventFilter, RedactionMode,
    RuntimeEnvConfig, runtime_env, runtime_env_for_plans,
};
pub use error::{SyncError, SyncResult};
pub use event::{
    DecisionEvent, PayloadEvent, SyncEvent, decode_event_line, encode_event_line, write_event_line,
};
pub use launch::{
    RuntimeLibraryPath, launch_command_for_plan, preload_env_value, run_with_preload,
    runtime_library_path, validate_native_backend_plan,
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
