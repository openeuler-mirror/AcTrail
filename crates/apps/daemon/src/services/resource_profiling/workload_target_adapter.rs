use ebpf_collector::procfs::ProcfsIdentityReader;
use model_core::{
    capability::Capability,
    ids::TraceId,
    process::{HostProcessCoordinates, MembershipState, ProcessIdentity},
    trace::TraceLifecycleState,
};
use process_identity::ProcessIdentityManager;
use resource_profiling::{ProfilingError, VerifiedProcessTarget, WorkloadTarget, WorkloadTargets};
use trace_runtime::{TraceRuntime, registry::TraceEntry};

pub(super) struct WorkloadTargetAdapter<'a> {
    traces: &'a TraceRuntime,
    identities: Option<&'a ProcessIdentityManager>,
}
impl<'a> WorkloadTargetAdapter<'a> {
    pub(super) fn new(
        traces: &'a TraceRuntime,
        identities: Option<&'a ProcessIdentityManager>,
    ) -> Self {
        Self { traces, identities }
    }
    pub(super) fn target(entry: &TraceEntry) -> WorkloadTarget {
        WorkloadTarget {
            trace_id: entry.trace.trace_id,
            root: entry.trace.root_process_identity,
            is_container: entry.trace.root_container_id.is_some(),
            requests_resources: entry.sensor_plan.collectors.iter().any(|collector| {
                collector
                    .capabilities
                    .contains(&Capability::ResourceMetrics)
            }),
            draining: entry.trace.lifecycle_state == TraceLifecycleState::Draining,
            terminal: entry.trace.lifecycle_state.is_terminal(),
        }
    }
}
impl WorkloadTargets for WorkloadTargetAdapter<'_> {
    fn traces(&self) -> Vec<WorkloadTarget> {
        self.traces
            .list_trace_records()
            .into_iter()
            .filter_map(|trace| self.traces.get_trace(trace.trace_id).map(Self::target))
            .collect()
    }
    fn contains_trace(&self, id: TraceId) -> bool {
        self.traces.get_trace(id).is_some()
    }
    fn coordinates(
        &self,
        identity: ProcessIdentity,
    ) -> Result<Option<HostProcessCoordinates>, ProfilingError> {
        let Some(mut host) = self
            .identities
            .and_then(|identities| identities.record(identity))
            .and_then(|record| record.host.clone())
        else {
            return Ok(None);
        };
        host.start_time_ticks = ProcfsIdentityReader
            .expected_start_time_ticks(&host)
            .ok_or_else(|| {
                ProfilingError::new(
                    "process_identity",
                    "expected process generation unavailable",
                )
            })?;
        Ok(Some(host))
    }
    fn process_targets(
        &self,
        target: &WorkloadTarget,
        include_children: bool,
    ) -> Vec<VerifiedProcessTarget> {
        let identities = if include_children {
            let Some(entry) = self.traces.get_trace(target.trace_id) else {
                return Vec::new();
            };
            entry
                .memberships
                .memberships()
                .filter(|m| {
                    m.capture_enabled
                        && matches!(m.state, MembershipState::Starting | MembershipState::Active)
                })
                .map(|m| m.identity)
                .collect::<Vec<_>>()
        } else {
            vec![target.root]
        };
        identities
            .into_iter()
            .filter_map(|identity| {
                let host = self.coordinates(identity).ok().flatten()?;
                Some(VerifiedProcessTarget::new(
                    identity,
                    host.pid,
                    host.start_time_ticks,
                ))
            })
            .collect()
    }
    fn root_is_gone(&self, root: ProcessIdentity) -> Result<bool, ProfilingError> {
        let Some(identities) = self.identities else {
            return Err(ProfilingError::new(
                "process_identity",
                "identity provider unavailable",
            ));
        };
        let Some(host) = identities
            .record(root)
            .and_then(|record| record.host.as_ref())
        else {
            return Ok(true);
        };
        let Some(expected) = ProcfsIdentityReader.expected_start_time_ticks(host) else {
            return Err(ProfilingError::new(
                "process_identity",
                "cannot establish expected process generation",
            ));
        };
        resource_profiling::ProcfsProcessLiveness::is_gone(&VerifiedProcessTarget::new(
            root, host.pid, expected,
        ))
        .map_err(|message| ProfilingError::new("resource_procfs_liveness", message))
    }
}
