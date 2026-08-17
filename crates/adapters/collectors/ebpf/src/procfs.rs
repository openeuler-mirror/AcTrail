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
/// Userspace, read once per container. `None` = host process or a runtime
/// layout the parser does not recognize. Pass the host pid (after NSpid
/// mapping) so the cgroup path carries the full runtime-assigned id.
pub fn read_container_identity(pid: u32) -> Option<ContainerIdentity> {
    let content = std::fs::read_to_string(format!("/proc/{pid}/cgroup")).ok()?;
    parse_container_identity(&content)
}

/// Parse a `/proc/<pid>/cgroup` file body into a container identity.
///
/// Recognizes Docker, containerd/Kata and Kubernetes cgroup layouts across
/// cgroup v1 (`N:controllers:/path`) and v2 (`0::/path`). A Kubernetes pod UID
/// ancestor is captured into `pod_uid`, never mistaken for the container id.
/// Kata guest cgroupfs layouts
/// (`[N:controllers|0::]/<containerd-namespace>/<container-id>`) are handled by
/// the final leaf fallback. The v2 form is verified from a real guest fixture;
/// the equivalent v1 multi-controller form is covered by regression fixtures.
/// Pure function: deterministic for a given input, no side effects.
pub fn parse_container_identity(cgroup_file: &str) -> Option<ContainerIdentity> {
    for line in cgroup_file.lines() {
        // "N:controllers:/path" (v1) or "0::/path" (v2); cgroup paths have no ':'.
        let Some(path) = line.splitn(3, ':').nth(2) else {
            continue;
        };
        if let Some(identity) = container_identity_from_path(path) {
            return Some(identity);
        }
    }
    None
}

/// Scope prefixes used by systemd cgroup drivers: `<prefix><id>.scope`.
const SCOPE_RUNTIMES: [(&str, ContainerRuntime); 2] = [
    ("docker-", ContainerRuntime::Docker),
    ("cri-containerd-", ContainerRuntime::Containerd),
];

fn container_identity_from_path(path: &str) -> Option<ContainerIdentity> {
    let mut pod_uid: Option<String> = None;
    let mut prev_was_docker = false;
    let mut prev_was_pod_dir = false;
    for segment in path.split('/') {
        // Kubernetes pod ancestor ("kubepods-…-pod<uid>.slice" or "pod<uid>"):
        // remember the UID for the container that follows, never take it as id.
        if let Some(uid) = pod_uid_from_segment(segment) {
            pod_uid = Some(uid);
            prev_was_pod_dir = true;
            prev_was_docker = false;
            continue;
        }
        // systemd driver: ".../<runtime>-<id>.scope"
        if let Some(scope) = segment.strip_suffix(".scope") {
            for (prefix, runtime) in SCOPE_RUNTIMES {
                if let Some(id) = scope.strip_prefix(prefix)
                    && is_container_id(id)
                {
                    let mut identity = ContainerIdentity::new(runtime, id);
                    identity.pod_uid = pod_uid;
                    return Some(identity);
                }
            }
        }
        // cgroupfs driver: ".../docker/<id>"
        if prev_was_docker && is_container_id(segment) {
            return Some(ContainerIdentity::new(ContainerRuntime::Docker, segment));
        }
        // cgroupfs CRI leaf: ".../kubepods/<qos>/pod<uid>/<id>". The bare hex
        // leaf does not say which runtime created it, so tag it `K8s`.
        if prev_was_pod_dir && is_container_id(segment) {
            let mut identity = ContainerIdentity::new(ContainerRuntime::K8s, segment);
            identity.pod_uid = pod_uid;
            return Some(identity);
        }
        prev_was_docker = segment == "docker";
        prev_was_pod_dir = false;
    }
    // Fallback: kata-agent (in-guest) lays workload cgroups out cgroupfs-style
    // as `/<containerd-namespace>/<container-id>` with no runtime-naming prefix
    // or `.scope` suffix. The v2 form is verified from a real Kata guest via
    // the debug console (e.g. `0::/k8s.io/<64-hex>`, `0::/default/<id>`); v1
    // exposes the same path once per controller.
    // None of the specific patterns above matched, so if the leaf itself is a
    // container id, tag it Containerd. The `is_container_id` shape (>=12 hex)
    // excludes guest system paths like `/init.scope` and
    // `/system.slice/kata-agent.service`.
    if let Some(leaf) = path.rsplit('/').find(|segment| !segment.is_empty())
        && is_container_id(leaf)
    {
        let mut identity = ContainerIdentity::new(ContainerRuntime::Containerd, leaf);
        identity.pod_uid = pod_uid;
        return Some(identity);
    }
    None
}

/// Extract a Kubernetes pod UID from a cgroup path segment, if present.
///
/// systemd driver encodes it as `kubepods-<qos->pod<uid_with_underscores>.slice`;
/// the cgroupfs driver as a plain `pod<uid>` directory.
fn pod_uid_from_segment(segment: &str) -> Option<String> {
    if segment.starts_with("kubepods")
        && let Some(rest) = segment.strip_suffix(".slice")
        && let Some(index) = rest.rfind("pod")
    {
        let uid = rest[index + 3..].replace('_', "-");
        if is_pod_uid(&uid) {
            return Some(uid);
        }
    }
    if let Some(raw) = segment.strip_prefix("pod")
        && is_pod_uid(raw)
    {
        return Some(raw.to_string());
    }
    None
}

/// Heuristic: does this cgroup file look containerized even though
/// [`parse_container_identity`] extracted nothing?
///
/// Security backstop, not an identity source. When a runtime lays out cgroups
/// in a shape the parser does not know (e.g. an unmapped kata-agent guest
/// layout), the process must NOT be silently treated as a host process —
/// callers use this to refuse host-level trust and to log the degradation
/// loudly instead. False positives only cost a warning plus stricter
/// matching; false negatives would silently weaken cross-container isolation.
pub fn cgroup_looks_containerized(cgroup_file: &str) -> bool {
    for line in cgroup_file.lines() {
        let Some(path) = line.splitn(3, ':').nth(2) else {
            continue;
        };
        if has_containerd_namespace_layout(path) {
            return true;
        }
        for segment in path.split('/') {
            if segment.is_empty() {
                continue;
            }
            let lower = segment.to_ascii_lowercase();
            // Unknown runtime markers stay a security-only backstop: they keep
            // unsupported container layouts from inheriting host trust, but
            // do not resolve a ContainerIdentity above.
            if lower.contains("docker")
                || lower.contains("containerd")
                || lower.contains("crio")
                || lower.contains("libpod")
                || lower.contains("kube")
                || lower.starts_with("kata")
                || lower == "vc"
                || pod_uid_from_segment(segment).is_some()
            {
                return true;
            }
        }
    }
    false
}

/// kata-agent/containerd may expose a workload as
/// `/<containerd-namespace>/<container-id>` without a runtime prefix. Keep
/// identity parsing strict, but treat even a non-hex leaf as containerized so
/// an unresolved workload cannot inherit host-level trust.
fn has_containerd_namespace_layout(path: &str) -> bool {
    let mut segments = path.split('/').filter(|segment| !segment.is_empty());
    matches!(segments.next(), Some("default" | "k8s.io")) && segments.next().is_some()
}

fn is_container_id(value: &str) -> bool {
    value.len() >= 12 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// RFC-4122 shape: 36 chars, hyphens at 8/13/18/23, hex elsewhere.
fn is_pod_uid(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 36
        && bytes.iter().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => *byte == b'-',
            _ => byte.is_ascii_hexdigit(),
        })
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
