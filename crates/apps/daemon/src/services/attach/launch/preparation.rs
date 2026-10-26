//! Prepare direct TLS probes before admitting a stopped launch trace.

use control_contract::command::{LaunchTlsProbePlan, TrackAddCommand};
use control_contract::reply::{ControlError, LaunchTlsPlanStatus};
use trace_runtime::TraceRuntime;

use super::{StorageAttachService, resolve_process_ref};
use crate::services::tls_sync::PinnedLaunchPlan;

impl StorageAttachService {
    pub(super) fn prepare_launch_tls(
        &mut self,
        trace_runtime: &mut TraceRuntime,
        command: &TrackAddCommand,
        host_ebpf: bool,
    ) -> Result<Vec<PinnedLaunchPlan>, ControlError> {
        let discover = host_ebpf
            && self.tls_sync.startup_discovery_enabled()
            && command.launch_target.is_some();
        if !discover && command.tls_probe_plans.is_empty() {
            return Ok(Vec::new());
        }
        let observation = resolve_process_ref(&command.root)?;
        let host_pid = observation
            .host
            .as_ref()
            .ok_or_else(|| ControlError::new("pid_resolution", "root host PID is missing"))?
            .pid;
        let mut plans = command.tls_probe_plans.clone();
        if discover {
            if let Some(target) = &command.launch_target {
                for binary in std::iter::once(&target.executable).chain(&target.candidates) {
                    let reply = self.tls_sync.resolve_launch_plan(binary, host_pid)?;
                    match reply.status {
                        LaunchTlsPlanStatus::Found(found) => {
                            for plan in found {
                                let plan = LaunchTlsProbePlan {
                                    target: plan.target,
                                    target_identity: plan.target_identity,
                                    binary: plan.binary,
                                    binary_identity: plan.binary_identity,
                                    provider: plan.provider,
                                    points: plan.points,
                                };
                                if !plans.contains(&plan) {
                                    plans.push(plan);
                                }
                            }
                        }
                        LaunchTlsPlanStatus::Unsupported { reason } => {
                            tracing::debug!(binary = %binary.display(), reason = reason.code(),
                                "Launch target has no direct TLS probe plan");
                        }
                    }
                    if let Err(error) = self.drain_live_events_impl(trace_runtime) {
                        tracing::warn!(code = %error.code, message = %error.message,
                            "Live event processing failed during launch preparation");
                    }
                }
            }
        }
        self.tls_sync.pin_launch_plans(host_pid, plans)
    }
}
