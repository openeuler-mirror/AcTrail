//! Active-trace registry ownership and indexing boundaries.

use std::collections::BTreeMap;
use std::time::SystemTime;

use collector_capability::CollectorDescriptor;
use model_core::ids::{OtelTraceId, TraceId, TraceName};
use model_core::process::{ExitStatus, MembershipState, ProcessIdentity, ProcessMembership};
use model_core::trace::{TraceAlertToken, TraceLifecycleState, TraceRecord};

use crate::commands::{RootRemovalRequest, TrackTraceRequest};
use crate::membership::MembershipIndex;
use crate::sensor_plan::{NegotiationFailure, SensorPlan};
use crate::state_machine;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceOwnerPrincipal {
    pub uid: u32,
    pub pid_namespace: String,
    pub mount_namespace: String,
    pub host_pid_namespace: bool,
    pub host_mount_namespace: bool,
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
    TraceIdentityGenerationFailed(String),
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
        let mut alert_token = [0_u8; TraceAlertToken::BYTE_COUNT];
        getrandom::fill(&mut alert_token)
            .map_err(|error| RegistryError::TraceIdentityGenerationFailed(error.to_string()))?;
        let mut otel_trace_id = [0_u8; OtelTraceId::BYTE_COUNT];
        getrandom::fill(&mut otel_trace_id)
            .map_err(|error| RegistryError::TraceIdentityGenerationFailed(error.to_string()))?;
        otel_trace_id[6] = (otel_trace_id[6] & 0x0f) | 0x40;
        otel_trace_id[8] = (otel_trace_id[8] & 0x3f) | 0x80;
        let otel_trace_id = OtelTraceId::from_bytes(otel_trace_id).ok_or_else(|| {
            RegistryError::TraceIdentityGenerationFailed(
                "generated OTLP trace identity is all zero".to_string(),
            )
        })?;
        let mut trace = TraceRecord::new(
            trace_id,
            otel_trace_id,
            TraceAlertToken::new(alert_token),
            request.root_identity.clone(),
            request.display_name,
            request.profile_snapshot.profile_name.clone(),
            request.created_at,
        );
        trace.root_pid_namespace = request.root_pid_namespace;
        trace.root_container_id = request.root_container_id;
        trace.root_pod_uid = request.root_pod_uid;
        trace.root_host_id = request.root_host_id;
        trace.root_working_directory = request.root_working_directory;
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
        entry.memberships.insert(membership);
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
        entry.memberships.insert(membership);
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
        entry.memberships.insert(membership);
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

    pub fn forget_trace(&mut self, trace_id: TraceId) -> Option<TraceEntry> {
        self.traces.remove(&trace_id)
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

    pub fn find_membership_in_trace(
        &self,
        trace_id: TraceId,
        identity: &ProcessIdentity,
    ) -> Option<ProcessMembership> {
        self.traces
            .get(&trace_id)?
            .memberships
            .get(identity)
            .cloned()
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
