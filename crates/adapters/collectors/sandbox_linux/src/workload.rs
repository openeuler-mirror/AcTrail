//! Guest workload cgroup sampling (Track B collector).

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use linux_cgroup::{
    CgroupCounters, CgroupV2CounterReader, parse_cgroup2_mount, parse_cgroup2_mount_id,
    parse_unified_process_cgroup,
};
use sandbox_observation::{
    GuestBootId, NormalizedContainerId, ProcessMarker, SandboxContainerRuntime,
    WorkloadCgroupCounters, WorkloadCgroupId, WorkloadCgroupResourceSnapshot,
};

use crate::SandboxLinuxError;
use crate::procfs::ProcfsReader;

/// Guest cgroup namespace layouts accepted by the workload sampler.
const GUEST_NAMESPACES: &[&str] = &["default", "k8s.io"];

/// One resolved container boundary inside the guest.
struct GuestWorkloadBoundary {
    runtime: SandboxContainerRuntime,
    container_id: NormalizedContainerId,
    boundary_relative_path: PathBuf,
}

pub struct WorkloadCgroupCollector {
    procfs: ProcfsReader,
    boot_id: GuestBootId,
    mount_point: PathBuf,
    mount_root: PathBuf,
    cgroup2_mount_id: u64,
    root_process_names: Vec<[u8; 16]>,
}

impl WorkloadCgroupCollector {
    pub fn start(
        procfs_root: PathBuf,
        root_process_names: Vec<[u8; 16]>,
    ) -> Result<Self, SandboxLinuxError> {
        let procfs = ProcfsReader::open(procfs_root)?;
        let boot_id = procfs.boot_id()?;
        let mountinfo = fs::read_to_string("/proc/self/mountinfo").map_err(|error| {
            SandboxLinuxError::new("workload_cgroup", format!("cannot read mountinfo: {error}"))
        })?;
        let (mount_root, mount_point) = parse_cgroup2_mount(&mountinfo)
            .map_err(|error| SandboxLinuxError::new("workload_cgroup", error.to_string()))?;
        let cgroup2_mount_id = parse_cgroup2_mount_id(&mountinfo)
            .map_err(|error| SandboxLinuxError::new("workload_cgroup", error.to_string()))?;
        Ok(Self {
            procfs,
            boot_id,
            mount_point,
            mount_root,
            cgroup2_mount_id,
            root_process_names,
        })
    }

    /// Emits one snapshot per distinct workload cgroup. A single workload whose
    /// boundary cannot be read is skipped without failing the remaining ones.
    pub fn sample(&self) -> Result<Vec<WorkloadCgroupResourceSnapshot>, SandboxLinuxError> {
        let roots = self.procfs.discover_roots(&self.root_process_names)?;
        if roots.is_empty() {
            return Ok(Vec::new());
        }
        let sampled_at_ms = now_ms()?;
        let mut workloads: BTreeMap<PathBuf, (GuestWorkloadBoundary, Vec<ProcessMarker>)> =
            BTreeMap::new();
        for root in roots {
            // Procfs is inherently racy. Resolution failure belongs to this root,
            // just like a counter failure belongs to its workload below.
            let Ok(Some(boundary)) = self.resolve_boundary(root) else {
                continue;
            };
            workloads
                .entry(boundary.boundary_relative_path.clone())
                .or_insert_with(|| (boundary, Vec::new()))
                .1
                .push(root);
        }
        let mut snapshots = Vec::with_capacity(workloads.len());
        for (boundary, roots) in workloads.values() {
            // Fail-local: a workload whose boundary cannot be read is skipped
            // without failing guest-wide sampling.
            if let Ok(snapshot) = self.read_workload(boundary, roots, sampled_at_ms) {
                snapshots.push(snapshot);
            }
        }
        snapshots.sort_unstable_by_key(|snapshot| snapshot.workload_id);
        Ok(snapshots)
    }

    fn resolve_boundary(
        &self,
        root: ProcessMarker,
    ) -> Result<Option<GuestWorkloadBoundary>, SandboxLinuxError> {
        let start_before = self
            .procfs
            .process_start_time_ticks(root.pid)
            .map_err(|error| {
                SandboxLinuxError::new(
                    "workload_cgroup",
                    format!(
                        "cannot verify process {} before cgroup read: {error}",
                        root.pid
                    ),
                )
            })?;
        if start_before != root.start_time_ticks {
            return Ok(None);
        }
        let path = self.procfs.root().join(root.pid.to_string()).join("cgroup");
        let raw = fs::read_to_string(&path).map_err(|error| {
            SandboxLinuxError::new(
                "workload_cgroup",
                format!("cannot read {}: {error}", path.display()),
            )
        })?;
        let start_after = self
            .procfs
            .process_start_time_ticks(root.pid)
            .map_err(|error| {
                SandboxLinuxError::new(
                    "workload_cgroup",
                    format!(
                        "cannot verify process {} after cgroup read: {error}",
                        root.pid
                    ),
                )
            })?;
        if start_after != root.start_time_ticks {
            return Ok(None);
        }
        let relative = parse_unified_process_cgroup(&raw)
            .map_err(|error| SandboxLinuxError::new("workload_cgroup", error.to_string()))?;
        parse_guest_boundary(&relative)
    }

    fn read_workload(
        &self,
        boundary: &GuestWorkloadBoundary,
        roots: &[ProcessMarker],
        sampled_at_ms: u64,
    ) -> Result<WorkloadCgroupResourceSnapshot, SandboxLinuxError> {
        let relative = boundary
            .boundary_relative_path
            .strip_prefix(&self.mount_root)
            .map_err(|_| {
                SandboxLinuxError::new("workload_cgroup", "boundary is outside cgroup mount root")
            })?;
        let boundary_path = self.mount_point.join(relative);
        let directory = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(&boundary_path)
            .map_err(|error| {
                SandboxLinuxError::new(
                    "workload_cgroup",
                    format!("cannot open {}: {error}", boundary_path.display()),
                )
            })?;
        self.read_open_workload(boundary, roots, sampled_at_ms, directory, &boundary_path)
    }

    fn read_open_workload(
        &self,
        boundary: &GuestWorkloadBoundary,
        roots: &[ProcessMarker],
        sampled_at_ms: u64,
        directory: fs::File,
        boundary_path: &Path,
    ) -> Result<WorkloadCgroupResourceSnapshot, SandboxLinuxError> {
        let metadata = directory.metadata().map_err(|error| {
            SandboxLinuxError::new(
                "workload_cgroup",
                format!("cannot stat {}: {error}", boundary_path.display()),
            )
        })?;
        let workload_id = WorkloadCgroupId::compute(
            &self.boot_id,
            self.cgroup2_mount_id,
            metadata.dev(),
            metadata.ino(),
            boundary.boundary_relative_path.as_os_str().as_bytes(),
        )
        .map_err(|error| SandboxLinuxError::new("workload_cgroup", error.to_string()))?;
        let mut reader = CgroupV2CounterReader::from_open_directory(directory, boundary_path)
            .map_err(|error| SandboxLinuxError::new("workload_cgroup", error.to_string()))?;
        let counters = reader
            .read_counters()
            .map_err(|error| SandboxLinuxError::new("workload_cgroup", error.to_string()))?;
        let representative_root = *roots.first().expect("workload has at least one root");
        let monitored_root_count = u32::try_from(roots.len()).map_err(|_| {
            SandboxLinuxError::new("workload_cgroup", "monitored root count exceeds u32")
        })?;
        let snapshot = WorkloadCgroupResourceSnapshot {
            guest_boot_id: self.boot_id,
            sampled_at_ms,
            workload_id,
            representative_root,
            monitored_root_count,
            runtime: boundary.runtime,
            container_id: Some(boundary.container_id),
            counters: map_counters(counters),
        };
        snapshot
            .validate()
            .map_err(|error| SandboxLinuxError::new("workload_cgroup", error))?;
        Ok(snapshot)
    }
}

fn parse_guest_boundary(
    relative_path: &Path,
) -> Result<Option<GuestWorkloadBoundary>, SandboxLinuxError> {
    let components = relative_path
        .components()
        .filter_map(|component| match component {
            Component::Normal(value) => value.to_str(),
            Component::RootDir => None,
            _ => None,
        })
        .collect::<Vec<_>>();
    if components.len() < 2 || !GUEST_NAMESPACES.contains(&components[0]) {
        return Ok(None);
    }
    let container_id = NormalizedContainerId::from_lower_hex(components[1])
        .map_err(|error| SandboxLinuxError::new("workload_cgroup", error))?;
    let boundary_relative_path = PathBuf::from("/").join(components[0]).join(components[1]);
    Ok(Some(GuestWorkloadBoundary {
        runtime: SandboxContainerRuntime::Containerd,
        container_id,
        boundary_relative_path,
    }))
}

fn map_counters(counters: CgroupCounters) -> WorkloadCgroupCounters {
    let memory_events = counters.memory_events;
    WorkloadCgroupCounters {
        memory_current_bytes: counters.memory_current_bytes,
        memory_peak_bytes: counters.memory_peak_bytes,
        memory_anon_bytes: counters.memory_anon_bytes,
        memory_file_bytes: counters.memory_file_bytes,
        memory_swap_current_bytes: counters.memory_swap_current_bytes,
        memory_low: memory_events.as_ref().and_then(|events| events.low),
        memory_high: memory_events.as_ref().and_then(|events| events.high),
        memory_max: memory_events.as_ref().and_then(|events| events.max),
        memory_oom: memory_events.as_ref().and_then(|events| events.oom),
        memory_oom_kill: memory_events.as_ref().and_then(|events| events.oom_kill),
        memory_oom_group_kill: memory_events
            .as_ref()
            .and_then(|events| events.oom_group_kill),
        cpu_usage_usec: Some(counters.cpu.usage_usec),
        cpu_user_usec: counters.cpu.user_usec,
        cpu_system_usec: counters.cpu.system_usec,
        cpu_nr_throttled: counters.cpu.nr_throttled,
        cpu_throttled_usec: counters.cpu.throttled_usec,
        io_read_bytes: counters.io.as_ref().and_then(|io| io.read_bytes),
        io_write_bytes: counters.io.as_ref().and_then(|io| io.write_bytes),
        pids_current: counters.pids_current,
        pids_peak: counters.pids_peak,
    }
}

fn now_ms() -> Result<u64, SandboxLinuxError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| SandboxLinuxError::new("workload_cgroup", error.to_string()))?
        .as_millis()
        .try_into()
        .map_err(|error| {
            SandboxLinuxError::new("workload_cgroup", format!("timestamp overflow: {error}"))
        })
}

#[cfg(test)]
#[path = "../../../../../tests/unit/crates/adapters/collectors/sandbox_linux/workload/tests.rs"]
mod tests;
