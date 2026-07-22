//! Existing-process attach sequencing and timing boundaries.

use std::collections::BTreeSet;
use std::time::SystemTime;

use collector_binding::CoverageGuardRequest;
use collector_instance::{CollectorError, CollectorInstance};
use config_core::trace_snapshot::CaptureProfileSnapshot;
use model_core::ids::{TraceId, TraceName};
use model_core::process::ProcessIdentity;
use process_identity::{IdentityLookupError, ProcessIdentityReader};
use process_identity::{ProcessIdentityError, ProcessIdentityManager};
use process_tree_snapshot_contract::snapshot::ProcessTreeSnapshotter;
use trace_runtime::commands::TrackTraceRequest;
use trace_runtime::registry::{RegistryError, TraceRuntime};
use trace_runtime::sensor_plan::SensorPlan;

use crate::coverage_guard::BootstrapTimings;
use crate::snapshot_merge::SnapshotMergeResult;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttachRequest {
    pub root_pid: u32,
    pub display_name: TraceName,
    pub profile_snapshot: CaptureProfileSnapshot,
    pub sensor_plan: SensorPlan,
    pub tags: BTreeSet<String>,
    pub request_received_at: SystemTime,
    pub capability_checked_at: SystemTime,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BootstrapResult {
    pub trace_id: TraceId,
    pub root_identity: ProcessIdentity,
    pub timings: BootstrapTimings,
    pub bootstrap_partial: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BootstrapError {
    IdentityLookup(IdentityLookupError),
    Collector(CollectorError),
    Snapshot(String),
    Runtime(RegistryError),
    ProcessIdentityManager(ProcessIdentityError),
}

pub struct BootstrapCoordinator<'a, C, I, S>
where
    C: CollectorInstance,
    I: ProcessIdentityReader,
    S: ProcessTreeSnapshotter,
{
    collector: &'a mut C,
    identity_reader: &'a I,
    snapshotter: &'a S,
    process_registry: &'a mut ProcessIdentityManager,
}

impl<'a, C, I, S> BootstrapCoordinator<'a, C, I, S>
where
    C: CollectorInstance,
    I: ProcessIdentityReader,
    S: ProcessTreeSnapshotter,
{
    pub fn new(
        collector: &'a mut C,
        identity_reader: &'a I,
        snapshotter: &'a S,
        process_registry: &'a mut ProcessIdentityManager,
    ) -> Self {
        Self {
            collector,
            identity_reader,
            snapshotter,
            process_registry,
        }
    }

    pub fn attach_existing(
        &mut self,
        runtime: &mut TraceRuntime,
        request: AttachRequest,
    ) -> Result<BootstrapResult, BootstrapError> {
        let root_observation = self
            .identity_reader
            .read_identity(request.root_pid)
            .map_err(BootstrapError::IdentityLookup)?;
        let root_identity = self
            .process_registry
            .resolve_or_create(root_observation.clone())
            .map_err(BootstrapError::ProcessIdentityManager)?
            .identity;

        let trace_id = runtime.reserve_trace_id();
        let guard = self
            .collector
            .install_coverage_guard(&CoverageGuardRequest {
                trace_id,
                root_identity: root_identity.clone(),
                root_observation: root_observation.clone(),
            })
            .map_err(BootstrapError::Collector)?;

        let snapshot_started_at = SystemTime::now();
        let snapshot = self
            .snapshotter
            .snapshot(&root_observation)
            .map_err(|_| BootstrapError::Snapshot("snapshot failed".to_string()))?;
        let snapshot_finished_at = SystemTime::now();
        let root_working_directory = snapshot.root_working_directory().map(str::to_string);
        runtime
            .create_starting_trace(
                trace_id,
                TrackTraceRequest {
                    root_identity: root_identity.clone(),
                    // Generic attach path: container id is resolved host-side in
                    // the daemon's `services/attach.rs`, not here.
                    root_container_id: None,
                    root_working_directory,
                    display_name: request.display_name,
                    profile_snapshot: request.profile_snapshot,
                    tags: request.tags,
                    created_at: request.request_received_at,
                },
                request.sensor_plan,
            )
            .map_err(BootstrapError::Runtime)?;
        let live_events = self
            .collector
            .poll_events()
            .map_err(BootstrapError::Collector)?;
        let SnapshotMergeResult {
            memberships,
            bootstrap_partial,
            process_records: _,
        } = crate::snapshot_merge::merge_snapshot(
            trace_id,
            &root_identity,
            &snapshot,
            &live_events,
            self.process_registry,
        )
        .map_err(BootstrapError::ProcessIdentityManager)?;

        for membership in memberships {
            runtime
                .insert_membership(trace_id, membership)
                .map_err(BootstrapError::Runtime)?;
        }

        let trace_started_at = snapshot_finished_at;
        runtime
            .activate_trace(trace_id, trace_started_at)
            .map_err(BootstrapError::Runtime)?;

        Ok(BootstrapResult {
            trace_id,
            root_identity,
            timings: BootstrapTimings {
                request_received_at: request.request_received_at,
                capability_checked_at: request.capability_checked_at,
                coverage_guard_installed_at: guard.activated_at,
                bootstrap_snapshot_started_at: snapshot_started_at,
                bootstrap_snapshot_finished_at: snapshot_finished_at,
                trace_started_at,
            },
            bootstrap_partial,
        })
    }
}
