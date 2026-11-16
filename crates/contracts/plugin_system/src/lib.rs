//! Shared plugin data contracts for runtimes and control-plane consumers.

mod http;
mod kind;
mod status;

pub use http::PluginHttpDeclaration;
pub use kind::{PluginPurpose, PluginRuntimeKind};
pub use status::{
    PluginHostcallMetrics, PluginInstanceStatus, PluginLifecycleState, PluginPayloadReadMetrics,
};
