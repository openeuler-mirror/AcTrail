//! `/proc`-backed helpers used for attach bootstrap and identity lookup.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::SystemTime;

use model_core::process::{NamespaceIdentity, ProcessIdentity};
use process_identity_contract::lookup::{IdentityLookupError, ProcessIdentityReader};
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
    fn read_identity(&self, pid: u32) -> Result<ProcessIdentity, IdentityLookupError> {
        let stat = read_stat(pid)?;
        let pid_namespace = read_pid_namespace(pid);
        Ok(
            ProcessIdentity::new(stat.pid, stat.start_time_ticks, stat.start_time_ticks)
                .with_namespace(pid_namespace),
        )
    }
}

pub fn resolve_namespaced_pid(
    namespace_pid: u32,
    pid_namespace: &NamespaceIdentity,
) -> Result<ProcessIdentity, String> {
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
            ProcessIdentity::new(stat.pid, stat.start_time_ticks, stat.start_time_ticks)
                .with_namespace(pid_namespace.clone()),
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

    fn snapshot(&self, root: &ProcessIdentity) -> Result<TreeSnapshot, Self::Error> {
        let stats = scan_proc_stats()?;
        if !stats.contains_key(&root.pid) {
            return Err(format!("root pid {} is not visible in /proc", root.pid));
        }

        let descendants = descendant_pids(root.pid, &stats);
        let mut processes = Vec::new();
        for pid in descendants {
            let Some(stat) = stats.get(&pid) else {
                continue;
            };
            let identity =
                ProcessIdentity::new(stat.pid, stat.start_time_ticks, stat.start_time_ticks)
                    .with_namespace(read_pid_namespace(stat.pid));
            let parent = if stat.pid == root.pid {
                None
            } else {
                stats.get(&stat.ppid).map(|parent| {
                    ProcessIdentity::new(
                        parent.pid,
                        parent.start_time_ticks,
                        parent.start_time_ticks,
                    )
                    .with_namespace(read_pid_namespace(parent.pid))
                })
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
    use process_identity_contract::lookup::ProcessIdentityReader;
    use process_tree_snapshot_contract::snapshot::ProcessTreeSnapshotter;

    use super::{ProcfsIdentityReader, ProcfsTreeSnapshotter};

    #[test]
    fn identity_reader_reads_current_process() {
        let identity = ProcfsIdentityReader
            .read_identity(std::process::id())
            .unwrap();
        assert_eq!(identity.pid, std::process::id());
        assert!(identity.start_time_ticks > 0);
    }

    #[test]
    fn tree_snapshot_contains_root_process() {
        let identity = ProcfsIdentityReader
            .read_identity(std::process::id())
            .unwrap();
        let snapshot = ProcfsTreeSnapshotter.snapshot(&identity).unwrap();
        assert!(
            snapshot
                .processes
                .iter()
                .any(|process| process.identity.pid == std::process::id())
        );
    }
}
