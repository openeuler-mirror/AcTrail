//! `/proc`-backed helpers used for attach bootstrap and identity lookup.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::SystemTime;

use model_core::container::{ContainerIdentity, ContainerRuntime};
use model_core::process::{
    HostProcessCoordinates, NamespaceIdentity, NamespaceProcessCoordinates, ProcessObservation,
};
use process_identity::{IdentityLookupError, ProcessIdentityReader};
use process_tree_snapshot_contract::snapshot::{
    ProcessSnapshot, ProcessTreeSnapshotter, TreeSnapshot,
};

#[derive(Clone, Debug, Eq, PartialEq)]
struct ProcStatRecord {
    pid: u32,
    ppid: u32,
    start_time_ticks: u64,
}

pub struct ProcfsIdentityReader;

impl ProcessIdentityReader for ProcfsIdentityReader {
    fn read_identity(&self, pid: u32) -> Result<ProcessObservation, IdentityLookupError> {
        let stat = read_stat(pid)?;
        let pid_namespace = read_pid_namespace(pid);
        Ok(
            ProcessObservation::host(HostProcessCoordinates::new(stat.pid, stat.start_time_ticks))
                .with_namespace(NamespaceProcessCoordinates::new(
                    pid_namespace,
                    read_nspid_last(pid).ok().flatten().unwrap_or(stat.pid),
                    stat.start_time_ticks,
                )),
        )
    }
}

pub fn resolve_namespaced_pid(
    namespace_pid: u32,
    pid_namespace: &NamespaceIdentity,
) -> Result<ProcessObservation, String> {
    if pid_namespace.as_str() == "unknown" {
        return Err("pid namespace is unknown; namespaced PID cannot be resolved".to_string());
    }

    let mut matches = Vec::new();
    for entry in std::fs::read_dir("/proc").map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let Ok(host_pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        if read_pid_namespace(host_pid) != *pid_namespace {
            continue;
        }
        if read_nspid_last(host_pid).ok().flatten() != Some(namespace_pid) {
            continue;
        }
        let Ok(stat) = read_stat(host_pid) else {
            continue;
        };
        matches.push(
            ProcessObservation::host(HostProcessCoordinates::new(stat.pid, stat.start_time_ticks))
                .with_namespace(NamespaceProcessCoordinates::new(
                    pid_namespace.clone(),
                    namespace_pid,
                    stat.start_time_ticks,
                )),
        );
    }

    match matches.as_slice() {
        [identity] => Ok(identity.clone()),
        [] => Err(format!(
            "no host process matched namespace pid {} in {}",
            namespace_pid,
            pid_namespace.as_str()
        )),
        _ => Err(format!(
            "multiple host processes matched namespace pid {} in {}",
            namespace_pid,
            pid_namespace.as_str()
        )),
    }
}

pub fn read_process_namespace_pid(pid: u32) -> Result<u32, String> {
    read_nspid_last(pid)?.ok_or_else(|| format!("process {pid} status does not expose NSpid"))
}

pub struct ProcfsTreeSnapshotter;

impl ProcessTreeSnapshotter for ProcfsTreeSnapshotter {
    type Error = String;

    fn snapshot(&self, root: &ProcessObservation) -> Result<TreeSnapshot, Self::Error> {
        let root_pid = root
            .host
            .as_ref()
            .map(|host| host.pid)
            .ok_or_else(|| "process tree snapshot requires a host PID".to_string())?;
        let stats = scan_proc_stats()?;
        if !stats.contains_key(&root_pid) {
            return Err(format!("root pid {root_pid} is not visible in /proc"));
        }

        let descendants = descendant_pids(root_pid, &stats);
        let mut processes = Vec::new();
        for pid in descendants {
            let Some(stat) = stats.get(&pid) else {
                continue;
            };
            let identity = process_observation(stat);
            let parent = if stat.pid == root_pid {
                None
            } else {
                stats.get(&stat.ppid).map(process_observation)
            };
            processes.push(ProcessSnapshot {
                identity,
                parent,
                // Snapshot-only enrichment for already-running processes.
                executable: read_link(stat.pid, "exe"),
                current_working_directory: read_link(stat.pid, "cwd"),
            });
        }

        Ok(TreeSnapshot {
            root: root.clone(),
            captured_at: SystemTime::now(),
            processes,
        })
    }
}

fn process_observation(stat: &ProcStatRecord) -> ProcessObservation {
    ProcessObservation::host(HostProcessCoordinates::new(stat.pid, stat.start_time_ticks))
        .with_namespace(NamespaceProcessCoordinates::new(
            read_pid_namespace(stat.pid),
            read_nspid_last(stat.pid).ok().flatten().unwrap_or(stat.pid),
            stat.start_time_ticks,
        ))
}

fn scan_proc_stats() -> Result<BTreeMap<u32, ProcStatRecord>, String> {
    let mut stats = BTreeMap::new();
    for entry in std::fs::read_dir("/proc").map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        if let Ok(stat) = read_stat(pid) {
            stats.insert(pid, stat);
        }
    }
    Ok(stats)
}

fn descendant_pids(root_pid: u32, stats: &BTreeMap<u32, ProcStatRecord>) -> BTreeSet<u32> {
    let mut descendants = BTreeSet::new();
    descendants.insert(root_pid);
    let mut changed = true;
    while changed {
        changed = false;
        for stat in stats.values() {
            if descendants.contains(&stat.ppid) && descendants.insert(stat.pid) {
                changed = true;
            }
        }
    }
    descendants
}

fn read_stat(pid: u32) -> Result<ProcStatRecord, IdentityLookupError> {
    let path = format!("/proc/{pid}/stat");
    let raw = std::fs::read_to_string(path).map_err(|error| {
        if proc_entry_gone(&error) {
            IdentityLookupError::NotFound { pid }
        } else if error.kind() == std::io::ErrorKind::PermissionDenied {
            IdentityLookupError::PermissionDenied { pid }
        } else {
            IdentityLookupError::Incomplete {
                pid,
                detail: error.to_string(),
            }
        }
    })?;
    let close_paren = raw
        .rfind(')')
        .ok_or_else(|| IdentityLookupError::Incomplete {
            pid,
            detail: "invalid /proc stat format".to_string(),
        })?;
    let remainder = raw
        .get(close_paren + 2..)
        .ok_or_else(|| IdentityLookupError::Incomplete {
            pid,
            detail: "missing stat fields".to_string(),
        })?;
    let fields = remainder.split_whitespace().collect::<Vec<_>>();
    let ppid = fields
        .get(1)
        .and_then(|value| value.parse::<u32>().ok())
        .ok_or_else(|| IdentityLookupError::Incomplete {
            pid,
            detail: "missing ppid".to_string(),
        })?;
    let start_time_ticks = fields
        .get(19)
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| IdentityLookupError::Incomplete {
            pid,
            detail: "missing start_time_ticks".to_string(),
        })?;
    Ok(ProcStatRecord {
        pid,
        ppid,
        start_time_ticks,
    })
}

fn read_pid_namespace(pid: u32) -> NamespaceIdentity {
    let path = PathBuf::from(format!("/proc/{pid}/ns/pid"));
    let value = std::fs::read_link(path)
        .map(|value| value.display().to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    NamespaceIdentity::new(value)
}

/// Resolve a process's container identity from its cgroup.
///
/// Userspace, read once per container. `None` = host process or non-Docker
/// runtime. Pass the host pid (after NSpid mapping) so
/// the cgroup path carries the full `docker-<id>`.
pub fn read_container_identity(pid: u32) -> Option<ContainerIdentity> {
    let content = std::fs::read_to_string(format!("/proc/{pid}/cgroup")).ok()?;
    parse_container_identity(&content)
}

/// Parse a `/proc/<pid>/cgroup` file body into a container identity.
///
/// Handles cgroup v2 (single `0::/path`) and v1 (`N:controllers:/path` lines).
/// Matches Docker (`docker-<id>.scope` systemd driver, or `/docker/<id>`
/// cgroupfs driver). Pure function for unit testing.
pub fn parse_container_identity(cgroup_file: &str) -> Option<ContainerIdentity> {
    for line in cgroup_file.lines() {
        // "N:controllers:/path" (v1) or "0::/path" (v2); cgroup paths have no ':'.
        let Some(path) = line.splitn(3, ':').nth(2) else {
            continue;
        };
        if let Some(id) = docker_id_from_path(path) {
            return Some(ContainerIdentity::new(ContainerRuntime::Docker, id));
        }
    }
    None
}

fn docker_id_from_path(path: &str) -> Option<String> {
    let mut prev_was_docker = false;
    for segment in path.split('/') {
        // systemd driver: ".../docker-<id>.scope"
        if let Some(id) = segment
            .strip_prefix("docker-")
            .and_then(|rest| rest.strip_suffix(".scope"))
            && is_container_id(id)
        {
            return Some(id.to_string());
        }
        // cgroupfs driver: ".../docker/<id>"
        if prev_was_docker && is_container_id(segment) {
            return Some(segment.to_string());
        }
        prev_was_docker = segment == "docker";
    }
    None
}

fn is_container_id(value: &str) -> bool {
    value.len() >= 12 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Map a container-internal file path to a host-reachable path.
///
/// Foundation for host-side file operations on container files (future
/// enforcement / rollback): the agent sees `/app/data.txt` inside its mount
/// namespace, but the host daemon must reach it through a live container
/// process's `/proc/<pid>/root` entry.
///
/// Resolved at use time, never pre-stored: the `/proc/<pid>/root` prefix is
/// only valid while that pid is alive, so a stored host path rots into a dangling
/// link as soon as the process exits. We re-pick a live anchor on every call.
///
/// `pid_namespace` identifies the container (1:1 with `container_id`). The trace's
/// root process may have exited, so we use any live member of the namespace as the
/// anchor. Returns `None` when:
/// - the path is not absolute (a relative path needs the process cwd to anchor -
///   out of scope here), or
/// - no live process remains in the namespace (container gone, so its overlay files
///   are reclaimed anyway, so there is nothing to map to).
pub fn resolve_host_path(
    pid_namespace: &NamespaceIdentity,
    container_internal_path: &str,
) -> Option<PathBuf> {
    if !container_internal_path.starts_with('/') {
        return None;
    }
    let anchor_pid = find_namespace_anchor_pid(pid_namespace)?;
    Some(host_path_via_anchor(anchor_pid, container_internal_path))
}

/// Pure path join: `/proc/<anchor_pid>/root/<path-without-leading-slash>`.
fn host_path_via_anchor(anchor_pid: u32, container_internal_path: &str) -> PathBuf {
    let relative = container_internal_path.trim_start_matches('/');
    PathBuf::from(format!("/proc/{anchor_pid}/root/{relative}"))
}

/// Find any live process in `pid_namespace` to use as a mount-namespace anchor.
fn find_namespace_anchor_pid(pid_namespace: &NamespaceIdentity) -> Option<u32> {
    if pid_namespace.as_str() == "unknown" {
        return None;
    }
    for entry in std::fs::read_dir("/proc").ok()?.flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        if read_pid_namespace(pid) == *pid_namespace {
            return Some(pid);
        }
    }
    None
}

fn read_nspid_last(pid: u32) -> Result<Option<u32>, String> {
    let raw = std::fs::read_to_string(format!("/proc/{pid}/status"))
        .map_err(|error| error.to_string())?;
    Ok(raw.lines().find_map(|line| {
        line.strip_prefix("NSpid:").and_then(|value| {
            value
                .split_whitespace()
                .last()
                .and_then(|raw| raw.parse::<u32>().ok())
        })
    }))
}

fn read_link(pid: u32, entry: &str) -> Option<String> {
    std::fs::read_link(format!("/proc/{pid}/{entry}"))
        .ok()
        .map(|value| value.display().to_string())
}

pub fn read_process_cwd(pid: u32) -> Option<String> {
    read_link(pid, "cwd")
}

fn proc_entry_gone(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::NotFound || error.raw_os_error() == Some(libc::ESRCH)
}

#[cfg(test)]
mod tests {
    use process_identity::ProcessIdentityReader;
    use process_tree_snapshot_contract::snapshot::ProcessTreeSnapshotter;

    use std::path::PathBuf;

    use model_core::container::ContainerRuntime;
    use model_core::process::NamespaceIdentity;

    use super::{
        ProcfsIdentityReader, ProcfsTreeSnapshotter, host_path_via_anchor,
        parse_container_identity, read_pid_namespace, resolve_host_path,
    };

    const ID: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn parse_container_docker_v2_systemd() {
        let cgroup = format!("0::/system.slice/docker-{ID}.scope\n");
        let identity = parse_container_identity(&cgroup).expect("docker id");
        assert_eq!(identity.runtime, ContainerRuntime::Docker);
        assert_eq!(identity.container_id, ID);
        assert!(identity.pod_uid.is_none());
    }

    #[test]
    fn parse_container_docker_v1_cgroupfs() {
        let cgroup = format!("12:pids:/docker/{ID}\n11:memory:/docker/{ID}\n");
        let identity = parse_container_identity(&cgroup).expect("docker id");
        assert_eq!(identity.runtime, ContainerRuntime::Docker);
        assert_eq!(identity.container_id, ID);
    }

    #[test]
    fn parse_container_host_is_none() {
        let cgroup = "0::/user.slice/user-1000.slice/session-3.scope\n";
        assert!(parse_container_identity(cgroup).is_none());
    }

    #[test]
    fn parse_container_garbage_is_none() {
        assert!(parse_container_identity("").is_none());
        assert!(parse_container_identity("no colons here\n").is_none());
        assert!(parse_container_identity("0::/system.slice/docker-short.scope\n").is_none());
    }

    #[test]
    fn host_path_via_anchor_strips_leading_slash() {
        assert_eq!(
            host_path_via_anchor(42, "/app/data.txt"),
            PathBuf::from("/proc/42/root/app/data.txt")
        );
        assert_eq!(
            host_path_via_anchor(7, "/a/b/c"),
            PathBuf::from("/proc/7/root/a/b/c")
        );
    }

    #[test]
    fn resolve_host_path_rejects_relative_path() {
        let ns = NamespaceIdentity::new("pid:[4026531836]");
        assert!(resolve_host_path(&ns, "relative/path").is_none());
    }

    #[test]
    fn resolve_host_path_rejects_unknown_namespace() {
        let ns = NamespaceIdentity::new("unknown");
        assert!(resolve_host_path(&ns, "/app/data.txt").is_none());
    }

    #[test]
    fn resolve_host_path_maps_through_live_anchor() {
        // The current process is a live member of its own pid namespace, so it
        // is always a valid anchor when namespaces are visible.
        let ns = read_pid_namespace(std::process::id());
        if ns.as_str() == "unknown" {
            return; // no namespace visibility in this environment
        }
        let mapped = resolve_host_path(&ns, "/etc/hostname").expect("live anchor exists");
        let rendered = mapped.to_string_lossy();
        assert!(rendered.starts_with("/proc/"));
        assert!(rendered.ends_with("/root/etc/hostname"));
    }

    #[test]
    fn identity_reader_reads_current_process() {
        let identity = ProcfsIdentityReader
            .read_identity(std::process::id())
            .unwrap();
        let host = identity.host.expect("host coordinates");
        assert_eq!(host.pid, std::process::id());
        assert!(host.start_time_ticks > 0);
    }

    #[test]
    fn tree_snapshot_contains_root_process() {
        let identity = ProcfsIdentityReader
            .read_identity(std::process::id())
            .unwrap();
        let snapshot = ProcfsTreeSnapshotter.snapshot(&identity).unwrap();
        assert!(snapshot.processes.iter().any(|process| {
            process
                .identity
                .host
                .as_ref()
                .is_some_and(|host| host.pid == std::process::id())
        }));
    }
}
