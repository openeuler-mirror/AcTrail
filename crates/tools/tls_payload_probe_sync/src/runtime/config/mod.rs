//! Runtime configuration loaded from launcher-provided environment variables.

mod codec;
mod factory;
mod plan;
mod policy;
mod state;

pub(super) use factory::RuntimeConfigFactory;
pub(super) use plan::{RuntimePlan, runtime_plan_for_binary};
pub(super) use state::{RuntimeConfig, get, set};
