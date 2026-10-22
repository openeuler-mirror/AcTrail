//! Client-side preparation for process injection.

#[path = "injection/noop.rs"]
mod noop;
#[path = "injection/tls_sync.rs"]
mod tls_sync;

use std::ffi::OsString;

use config_core::daemon::PayloadTlsConfig;
use control_contract::command::LaunchTlsProbePlan;
use model_core::ids::{RequestId, TraceId};

use super::suppress::InheritableSuppressedFd;
use super::timing::LaunchTiming;
use crate::transport::ControlClientPort;

pub(super) struct PreparationContext<'a> {
    pub client: &'a mut dyn ControlClientPort,
    pub request_id: RequestId,
    pub config: &'a PayloadTlsConfig,
    pub agent_commands: &'a [String],
    pub timing: &'a mut LaunchTiming,
}

pub(super) struct EnvironmentContext<'a> {
    pub trace_id: TraceId,
    pub config: &'a PayloadTlsConfig,
    pub socket_max_segment_bytes: u32,
    pub event_fd: Option<&'a InheritableSuppressedFd>,
    pub timing: &'a mut LaunchTiming,
}

#[derive(Default)]
pub(super) struct InjectionResources {
    pub event_fd: Option<InheritableSuppressedFd>,
    pub direct_probe_plans: Vec<LaunchTlsProbePlan>,
}

pub(super) trait LaunchInjection {
    fn prepare(
        &mut self,
        command: &mut Vec<OsString>,
        context: PreparationContext<'_>,
    ) -> Result<InjectionResources, String>;

    fn envs(&self, context: EnvironmentContext<'_>) -> Result<Vec<(OsString, OsString)>, String>;
}

pub(super) struct InjectionFactory;

impl InjectionFactory {
    pub fn create(config: &PayloadTlsConfig) -> Box<dyn LaunchInjection> {
        if config.enabled && config.capture_backend.is_sync() {
            Box::new(tls_sync::TlsSyncInjection::default())
        } else {
            Box::new(noop::NoopInjection)
        }
    }
}
