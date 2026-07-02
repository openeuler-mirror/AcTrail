//! Active-trace registry ownership and indexing boundaries.

use std::collections::BTreeMap;
use std::time::SystemTime;

use collector_capability::CollectorDescriptor;
use model_core::ids::{TraceId, TraceName};
use model_core::process::{ExitStatus, MembershipState, ProcessIdentity, ProcessMembership};
use model_core::trace::{TraceLifecycleState, TraceRecord};

use crate::commands::{RootRemovalRequest, TrackTraceRequest};
use crate::membership::{MembershipIndex, MembershipInsertResult, MembershipRefreshResult};
use crate::sensor_plan::{NegotiationFailure, SensorPlan};
use crate::state_machine;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceOwnerPrincipal {
    pub uid: u32,
    pub container_id: Option<String>,
    pub pid_namespace: String,
    pub host_pid_namespace: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceEntry {
    pub trace: TraceRecord,
    pub profile_snapshot: config_core::trace_snapshot::CaptureProfileSnapshot,
    pub sensor_plan: SensorPlan,
    pub memberships: MembershipIndex,
    pub owner: Option<TraceOwnerPrincipal>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RegistryError {
    TraceNotFound(TraceId),
    NegotiationFailed(Vec<NegotiationFailure>),
    RootMembershipMissing(TraceId),
    OwnerAlreadyBound(TraceId),
    ParentMembershipMissing(ProcessIdentity),
    PropagationDisabled(ProcessIdentity),
    InvalidStateTransition(state_machine::StateTransitionError),
}

pub struct TraceRuntime {
    next_trace_id: u64,
    collectors: Vec<CollectorDescriptor>,
    traces: BTreeMap<TraceId, TraceEntry>,
}

impl TraceRuntime {
    pub fn new(collectors: Vec<CollectorDescriptor>, initial_trace_id: u64) -> Self {
        Self {
            next_trace_id: initial_trace_id,
            collectors,
            traces: BTreeMap::new(),
        }
    }

    pub fn reserve_trace_id(&mut self) -> TraceId {
        let trace_id = TraceId::new(self.next_trace_id);
        self.next_trace_id += 1;
        trace_id
    }

    pub fn negotiate(
        &self,
        snapshot: &config_core::trace_snapshot::CaptureProfileSnapshot,
    ) -> Result<SensorPlan, RegistryError> {
        SensorPlan::negotiate(snapshot, &self.collectors).map_err(RegistryError::NegotiationFailed)
    }

    pub fn create_starting_trace(
        &mut self,
        trace_id: TraceId,
        request: TrackTraceRequest,
        sensor_plan: SensorPlan,
    ) -> Result<(), RegistryError> {
        let mut trace = TraceRecord::new(
            trace_id,
            request.root_identity.clone(),
            request.display_name,
            request.profile_snapshot.profile_name.clone(),
            request.created_at,
        );
        trace.root_container_id = request.root_container_id;
        for tag in request.tags {
            trace.add_tag(tag);
        }

        let root_membership =
            ProcessMembership::root(trace_id, request.root_identity, request.created_at);
        let memberships = MembershipIndex::new(root_membership);
        self.traces.insert(
            trace_id,
            TraceEntry {
                trace,
                profile_snapshot: request.profile_snapshot,
                sensor_plan,
                memberships,
                owner: None,
            },
        );
        Ok(())
    }

    pub fn bind_trace_owner(
        &mut self,
        trace_id: TraceId,
        owner: TraceOwnerPrincipal,
    ) -> Result<(), RegistryError> {
        let entry = self
            .traces
            .get_mut(&trace_id)
            .ok_or(RegistryError::TraceNotFound(trace_id))?;
        if entry.owner.is_some() {
            return Err(RegistryError::OwnerAlreadyBound(trace_id));
        }
        entry.owner = Some(owner);
        Ok(())
    }

    pub fn activate_trace(
        &mut self,
        trace_id: TraceId,
        started_at: SystemTime,
    ) -> Result<(), RegistryError> {
        let entry = self
            .traces
            .get_mut(&trace_id)
            .ok_or(RegistryError::TraceNotFound(trace_id))?;
        entry.memberships.activate_all();
        state_machine::start_trace(&mut entry.trace, started_at)
            .map_err(RegistryError::InvalidStateTransition)
    }

    pub fn insert_membership(
        &mut self,
        trace_id: TraceId,
        membership: ProcessMembership,
    ) -> Result<(), RegistryError> {
        let entry = self
            .traces
            .get_mut(&trace_id)
            .ok_or(RegistryError::TraceNotFound(trace_id))?;
        if let MembershipInsertResult::PidReused { stale_identity } =
            entry.memberships.insert(membership)
        {
            if let Some(stale) = entry.memberships.get_mut(&stale_identity) {
                stale.mark_identity_stale();
            }
            state_machine::degrade_trace(&mut entry.trace);
        }
        Ok(())
    }

    pub fn inherit_process(
        &mut self,
        trace_id: TraceId,
        parent_identity: &ProcessIdentity,
        child_identity: ProcessIdentity,
        observed_at: SystemTime,
    ) -> Result<(), RegistryError> {
        let entry = self
            .traces
            .get_mut(&trace_id)
            .ok_or(RegistryError::TraceNotFound(trace_id))?;
        let parent = entry
            .memberships
            .get(parent_identity)
            .cloned()
            .ok_or_else(|| RegistryError::ParentMembershipMissing(parent_identity.clone()))?;
        if !parent.can_inherit() {
            return Err(RegistryError::PropagationDisabled(parent.identity));
        }

        let membership = ProcessMembership::inherited(
            trace_id,
            child_identity,
            parent.identity.clone(),
            observed_at,
        );
        let _ = entry.memberships.insert(membership);
        Ok(())
    }

    pub fn insert_observed_child(
        &mut self,
        trace_id: TraceId,
        parent_identity: &ProcessIdentity,
        child_identity: ProcessIdentity,
        observed_at: SystemTime,
    ) -> Result<(), RegistryError> {
        let entry = self
            .traces
            .get_mut(&trace_id)
            .ok_or(RegistryError::TraceNotFound(trace_id))?;
        let parent = entry
            .memberships
            .get(parent_identity)
            .cloned()
            .ok_or_else(|| RegistryError::ParentMembershipMissing(parent_identity.clone()))?;
        if !parent.capture_enabled
            || !parent.propagation_enabled
            || matches!(parent.state, MembershipState::IdentityStale)
        {
            return Err(RegistryError::PropagationDisabled(parent.identity));
        }

        let membership = ProcessMembership::inherited(
            trace_id,
            child_identity,
            parent.identity.clone(),
            observed_at,
        );
        let _ = entry.memberships.insert(membership);
        Ok(())
    }

    pub fn track_remove_root(&mut self, request: RootRemovalRequest) -> Result<(), RegistryError> {
        let entry = self
            .traces
            .get_mut(&request.trace_id)
            .ok_or(RegistryError::TraceNotFound(request.trace_id))?;
        let root_identity = entry.trace.root_process_identity.clone();
        let root = entry
            .memberships
            .get_mut(&root_identity)
            .ok_or(RegistryError::RootMembershipMissing(request.trace_id))?;
        root.disable_capture();
        root.disable_propagation();
        self.reconcile_lifecycle(request.trace_id, request.removed_at)
    }

    pub fn mark_process_exited(
        &mut self,
        trace_id: TraceId,
        identity: &ProcessIdentity,
        status: ExitStatus,
    ) -> Result<(), RegistryError> {
        let entry = self
            .traces
            .get_mut(&trace_id)
            .ok_or(RegistryError::TraceNotFound(trace_id))?;
        let membership = entry
            .memberships
            .get_mut(identity)
            .ok_or_else(|| RegistryError::ParentMembershipMissing(identity.clone()))?;
        let observed_at = status.observed_at;
        membership.mark_exited(status);
        self.reconcile_lifecycle(trace_id, observed_at)
    }

    pub fn mark_degraded(&mut self, trace_id: TraceId) -> Result<(), RegistryError> {
        let entry = self
            .traces
            .get_mut(&trace_id)
            .ok_or(RegistryError::TraceNotFound(trace_id))?;
        state_machine::degrade_trace(&mut entry.trace);
        Ok(())
    }

    pub fn fail_trace(
        &mut self,
        trace_id: TraceId,
        failed_at: SystemTime,
    ) -> Result<(), RegistryError> {
        let entry = self
            .traces
            .get_mut(&trace_id)
            .ok_or(RegistryError::TraceNotFound(trace_id))?;
        state_machine::fail_trace(&mut entry.trace, failed_at)
            .map_err(RegistryError::InvalidStateTransition)
    }

    pub fn get_trace(&self, trace_id: TraceId) -> Option<&TraceEntry> {
        self.traces.get(&trace_id)
    }

    pub fn find_membership(
        &self,
        identity: &ProcessIdentity,
    ) -> Option<(TraceId, ProcessMembership)> {
        self.traces.iter().find_map(|(trace_id, entry)| {
            entry
                .memberships
                .get(identity)
                .cloned()
                .map(|membership| (*trace_id, membership))
        })
    }

    pub fn find_membership_by_pid(&self, pid: u32) -> Option<(TraceId, ProcessMembership)> {
        self.traces.iter().find_map(|(trace_id, entry)| {
            entry
                .memberships
                .by_pid(pid)
                .cloned()
                .map(|membership| (*trace_id, membership))
        })
    }

    pub fn refresh_process_identity(
        &mut self,
        refreshed_identity: ProcessIdentity,
    ) -> Option<(TraceId, ProcessIdentity)> {
        for (trace_id, entry) in &mut self.traces {
            match entry
                .memberships
                .refresh_active_pid_identity(refreshed_identity.clone())
            {
                MembershipRefreshResult::Missing => {}
                MembershipRefreshResult::Unchanged => {
                    return Some((*trace_id, refreshed_identity));
                }
                MembershipRefreshResult::Refreshed { ref stale_identity } => {
                    if entry.trace.root_process_identity == *stale_identity {
                        entry.trace.root_process_identity = refreshed_identity.clone();
                    }
                    return Some((*trace_id, refreshed_identity));
                }
            }
        }
        None
    }

    pub fn list_trace_records(&self) -> Vec<&TraceRecord> {
        self.traces.values().map(|entry| &entry.trace).collect()
    }

    pub fn find_trace_by_name(&self, name: &TraceName) -> Option<&TraceEntry> {
        self.traces
            .values()
            .find(|entry| entry.trace.display_name == *name)
    }

    fn reconcile_lifecycle(
        &mut self,
        trace_id: TraceId,
        observed_at: SystemTime,
    ) -> Result<(), RegistryError> {
        let entry = self
            .traces
            .get_mut(&trace_id)
            .ok_or(RegistryError::TraceNotFound(trace_id))?;
        if entry.trace.lifecycle_state.is_terminal() {
            return Ok(());
        }

        let root_identity = entry.trace.root_process_identity.clone();
        let root = entry
            .memberships
            .get(&root_identity)
            .ok_or(RegistryError::RootMembershipMissing(trace_id))?;
        let active_descendants = entry.memberships.active_descendants_of(&root_identity);

        if !root.capture_enabled
            || matches!(root.state, model_core::process::MembershipState::Exited)
        {
            if active_descendants > 0 {
                if entry.trace.lifecycle_state == TraceLifecycleState::Active {
                    state_machine::begin_draining(&mut entry.trace, observed_at)
                        .map_err(RegistryError::InvalidStateTransition)?;
                }
            } else if entry.memberships.capturable_members() == 0 {
                if matches!(root.state, MembershipState::Exited) {
                    state_machine::exit_trace(&mut entry.trace, observed_at)
                        .map_err(RegistryError::InvalidStateTransition)?;
                } else {
                    state_machine::complete_trace(&mut entry.trace, observed_at)
                        .map_err(RegistryError::InvalidStateTransition)?;
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::time::SystemTime;

    use config_core::capture_profile::CaptureProfile;
    use collector_capability::CollectorDescriptor;
    use config_core::trace_snapshot::CaptureProfileSnapshot;
    use model_core::capability::{Capability, CapabilityRequest, RequestMode};
    use model_core::ids::{CollectorName, ProfileName, TraceName};
    use model_core::process::{ExitStatus, ProcessIdentity};
    use model_core::trace::TraceLifecycleState;

    use crate::TraceRuntime;
    use crate::commands::{RootRemovalRequest, TrackTraceRequest};
    use crate::sensor_plan::SensorPlan;

    fn runtime() -> TraceRuntime {
        TraceRuntime::new(
            vec![CollectorDescriptor {
                name: CollectorName::new("ebpf"),
                capabilities: vec![model_core::capability::CapabilityDescriptor::new(
                    Capability::ProcLifecycle,
                    Vec::new(),
                )],
                supports_attach_coverage_guard: true,
                supports_existing_pid_attach: true,
            }],
            1,
        )
    }

    fn profile_snapshot() -> CaptureProfileSnapshot {
        let profile = CaptureProfile::new(
            ProfileName::new("default"),
            vec![CapabilityRequest::new(
                Capability::ProcLifecycle,
                RequestMode::Required,
            )],
        );
        CaptureProfileSnapshot::from_profile(&profile, SystemTime::UNIX_EPOCH)
    }

    #[test]
    fn track_remove_keeps_trace_draining_when_descendant_exists() {
        let mut runtime = runtime();
        let trace_id = runtime.reserve_trace_id();
        let root = ProcessIdentity::new(100, 1, 1);
        let request = TrackTraceRequest {
            root_identity: root.clone(),
            root_container_id: None,
            display_name: TraceName::new("agent"),
            profile_snapshot: profile_snapshot(),
            tags: BTreeSet::new(),
            created_at: SystemTime::UNIX_EPOCH,
        };
        let plan = SensorPlan::negotiate(&request.profile_snapshot, &runtime.collectors).unwrap();

        runtime
            .create_starting_trace(trace_id, request, plan)
            .unwrap();
        runtime
            .activate_trace(trace_id, SystemTime::UNIX_EPOCH)
            .unwrap();
        runtime
            .inherit_process(
                trace_id,
                &root,
                ProcessIdentity::new(101, 2, 1),
                SystemTime::UNIX_EPOCH,
            )
            .unwrap();
        runtime
            .track_remove_root(RootRemovalRequest {
                trace_id,
                removed_at: SystemTime::UNIX_EPOCH,
            })
            .unwrap();

        let entry = runtime.get_trace(trace_id).unwrap();
        assert_eq!(entry.trace.lifecycle_state, TraceLifecycleState::Draining);
    }

    #[test]
    fn root_exit_marks_trace_exited_without_descendants() {
        let mut runtime = runtime();
        let trace_id = runtime.reserve_trace_id();
        let root = ProcessIdentity::new(100, 1, 1);
        let request = TrackTraceRequest {
            root_identity: root.clone(),
            root_container_id: None,
            display_name: TraceName::new("agent"),
            profile_snapshot: profile_snapshot(),
            tags: BTreeSet::new(),
            created_at: SystemTime::UNIX_EPOCH,
        };
        let plan = SensorPlan::negotiate(&request.profile_snapshot, &runtime.collectors).unwrap();

        runtime
            .create_starting_trace(trace_id, request, plan)
            .unwrap();
        runtime
            .activate_trace(trace_id, SystemTime::UNIX_EPOCH)
            .unwrap();
        runtime
            .mark_process_exited(
                trace_id,
                &root,
                ExitStatus {
                    code: Some(0),
                    observed_at: SystemTime::UNIX_EPOCH,
                    source: Some(model_core::process::ExitObservationSource::Event),
                },
            )
            .unwrap();

        let entry = runtime.get_trace(trace_id).unwrap();
        assert_eq!(entry.trace.lifecycle_state, TraceLifecycleState::Exited);
    }

    #[test]
    fn root_exec_refresh_updates_trace_root_identity() {
        let mut runtime = runtime();
        let trace_id = runtime.reserve_trace_id();
        let root_before_exec = ProcessIdentity::new(100, 1, 1);
        let root_after_exec = ProcessIdentity::new(100, 1, 2);
        let request = TrackTraceRequest {
            root_identity: root_before_exec.clone(),
            root_container_id: None,
            display_name: TraceName::new("agent"),
            profile_snapshot: profile_snapshot(),
            tags: BTreeSet::new(),
            created_at: SystemTime::UNIX_EPOCH,
        };
        let plan = SensorPlan::negotiate(&request.profile_snapshot, &runtime.collectors).unwrap();

        runtime
            .create_starting_trace(trace_id, request, plan)
            .unwrap();
        runtime
            .activate_trace(trace_id, SystemTime::UNIX_EPOCH)
            .unwrap();
        runtime.refresh_process_identity(root_after_exec.clone());

        let entry = runtime.get_trace(trace_id).unwrap();
        assert_eq!(entry.trace.root_process_identity, root_after_exec);
        assert_eq!(
            entry.memberships.get(&root_before_exec).unwrap().state,
            model_core::process::MembershipState::IdentityStale
        );
    }
}
