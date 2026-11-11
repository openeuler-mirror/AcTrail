//! Builds the coordinator's launch request for a Trace that `TrackAdd` just attached.

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::time::SystemTime;

use checkpoint::TraceProcess;
use checkpoint_core::LaunchRequest;
use checkpoint_core::model::{ScopeConfig, SnapshotContext, TraceOwner};
use control_contract::checkpoint::LaunchCheckpointConfig;
use control_contract::reply::ControlError;
use model_core::ids::TraceId;
use trace_runtime::TraceOwnerPrincipal;

use super::procfs;

/// The root is still stopped at the launch barrier, so its `/proc` entries are stable here.
pub(crate) fn launch_request(
    trace_id: TraceId,
    owner: &TraceOwnerPrincipal,
    root_host_pid: u32,
    config: LaunchCheckpointConfig,
) -> Result<LaunchRequest, ControlError> {
    require_owned_scopes(owner, &config)?;
    let root_error = |error: std::io::Error| {
        ControlError::new(
            "checkpoint_launch",
            format!("read the Trace root {root_host_pid} in /proc: {error}"),
        )
    };
    let stat = procfs::read_stat(root_host_pid)
        .map_err(root_error)?
        .ok_or_else(|| {
            ControlError::new(
                "checkpoint_launch",
                format!("the Trace root {root_host_pid} has exited"),
            )
        })?;
    let trace_process = TraceProcess {
        host_pid: root_host_pid,
        start_time_ticks: stat.start_time_ticks,
        pid_namespace: procfs::read_link(root_host_pid, "ns/pid")
            .map_err(root_error)?
            .into(),
        mount_namespace: procfs::read_link(root_host_pid, "ns/mnt")
            .map_err(root_error)?
            .into(),
        cwd: procfs::read_link(root_host_pid, "cwd")
            .map_err(root_error)?
            .into(),
    };
    Ok(LaunchRequest {
        context: SnapshotContext { trace_id },
        owner: TraceOwner {
            uid: owner.uid,
            pid_namespace: owner.pid_namespace.clone(),
            mount_namespace: owner.mount_namespace.clone(),
            host_pid_namespace: owner.host_pid_namespace,
            host_mount_namespace: owner.host_mount_namespace,
        },
        trace_process,
        scopes: config
            .scopes
            .into_iter()
            .map(|scope| ScopeConfig {
                id: scope.id,
                root: scope.root,
                excludes: scope.excludes,
                backend_instance: scope.backend_instance,
            })
            .collect(),
        requested_at: SystemTime::now(),
    })
}

/// Backends save and restore as root, so a user other than host root may only protect
/// directories it owns; otherwise a restore would let it put back another user's files.
fn require_owned_scopes(
    owner: &TraceOwnerPrincipal,
    config: &LaunchCheckpointConfig,
) -> Result<(), ControlError> {
    if owner.uid == 0 && owner.host_pid_namespace && owner.host_mount_namespace {
        return Ok(());
    }
    for scope in &config.scopes {
        let metadata = fs::metadata(&scope.root).map_err(|error| {
            ControlError::new(
                "checkpoint_launch",
                format!("read scope root {}: {error}", scope.root),
            )
        })?;
        if metadata.uid() != owner.uid {
            return Err(ControlError::new(
                "scope_not_owned",
                format!(
                    "scope {} root {} belongs to uid {}, not to the launching uid {}",
                    scope.id,
                    scope.root,
                    metadata.uid(),
                    owner.uid
                ),
            ));
        }
    }
    Ok(())
}
