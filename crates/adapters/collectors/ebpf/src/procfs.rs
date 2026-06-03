//! `/proc`-backed helpers used for attach bootstrap and identity lookup.

use std::collections::{BTreeMap, BTreeSet};
use std::net::{Ipv4Addr, Ipv6Addr};
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::time::SystemTime;

use model_core::process::{NamespaceIdentity, ProcessIdentity};
use process_identity_contract::lookup::{IdentityLookupError, ProcessIdentityReader};
use process_tree_snapshot_contract::snapshot::{
    ProcessSnapshot, ProcessTreeSnapshotter, TreeSnapshot,
};

use crate::loader::PidNamespace;

#[path = "procfs/fd.rs"]
mod fd;

pub use fd::{FdObservation, FdTargetKind, resolve_fd_observation};

#[derive(Clone, Debug, Eq, PartialEq)]
struct ProcStatRecord {
    pid: u32,
    ppid: u32,
    start_time_ticks: u64,
    start_unix_seconds: Option<u64>,
}

// CLK_TCK is typically 100 on Linux
pub const CLK_TCK: u64 = 100;

pub fn read_boot_time() -> Option<u64> {
    let content = std::fs::read_to_string("/proc/stat").ok()?;
    for line in content.lines() {
        if let Some(value) = line.strip_prefix("btime ") {
            return value.trim().parse::<u64>().ok();
        }
    }
    None
}

pub struct ProcfsIdentityReader;

impl ProcessIdentityReader for ProcfsIdentityReader {
    fn read_identity(&self, pid: u32) -> Result<ProcessIdentity, IdentityLookupError> {
        let stat = read_stat(pid)?;
        let pid_namespace = read_pid_namespace(pid);
        let mut identity =
            ProcessIdentity::new(stat.pid, stat.start_time_ticks, stat.start_time_ticks)
                .with_namespace(pid_namespace);
        if let Some(start_unix) = stat.start_unix_seconds {
            identity = identity.with_start_unix(start_unix);
        }
        Ok(identity)
    }
}

pub struct ProcfsTreeSnapshotter;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SocketEndpointObservation {
    pub transport: String,
    pub local: Option<String>,
    pub remote: Option<String>,
}

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
            let mut identity =
                ProcessIdentity::new(stat.pid, stat.start_time_ticks, stat.start_time_ticks)
                    .with_namespace(read_pid_namespace(stat.pid));
            if let Some(start_unix) = stat.start_unix_seconds {
                identity = identity.with_start_unix(start_unix);
            }
            let parent = if stat.pid == root.pid {
                None
            } else {
                stats.get(&stat.ppid).map(|parent_stat| {
                    let mut parent_identity = ProcessIdentity::new(
                        parent_stat.pid,
                        parent_stat.start_time_ticks,
                        parent_stat.start_time_ticks,
                    )
                    .with_namespace(read_pid_namespace(parent_stat.pid));
                    if let Some(start_unix) = parent_stat.start_unix_seconds {
                        parent_identity = parent_identity.with_start_unix(start_unix);
                    }
                    parent_identity
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
        if error.kind() == std::io::ErrorKind::NotFound {
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
    
    // Calculate start_unix_seconds from boot_time + ticks
    let start_unix_seconds = read_boot_time().map(|boot_time| {
        boot_time + (start_time_ticks / CLK_TCK)
    });
    
    Ok(ProcStatRecord {
        pid,
        ppid,
        start_time_ticks,
        start_unix_seconds,
    })
}

pub fn read_process_start_unix(pid: u32) -> Option<u64> {
    let boot_time = read_boot_time()?;
    let path = format!("/proc/{pid}/stat");
    let raw = std::fs::read_to_string(path).ok()?;
    let close_paren = raw.rfind(')')?;
    let remainder = raw.get(close_paren + 2..)?;
    let fields = remainder.split_whitespace().collect::<Vec<_>>();
    let start_time_ticks = fields.get(19)?.parse::<u64>().ok()?;
    Some(boot_time + (start_time_ticks / CLK_TCK))
}

fn read_pid_namespace(pid: u32) -> NamespaceIdentity {
    let path = PathBuf::from(format!("/proc/{pid}/ns/pid"));
    let value = std::fs::read_link(path)
        .map(|value| value.display().to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    NamespaceIdentity::new(value)
}

pub fn read_pid_namespace_handle(pid: u32) -> Result<PidNamespace, String> {
    let metadata =
        std::fs::metadata(format!("/proc/{pid}/ns/pid")).map_err(|error| error.to_string())?;
    Ok(PidNamespace {
        dev: metadata.dev(),
        ino: metadata.ino(),
    })
}

fn read_link(pid: u32, entry: &str) -> Option<String> {
    std::fs::read_link(format!("/proc/{pid}/{entry}"))
        .ok()
        .map(|value| value.display().to_string())
}

pub fn read_process_cwd(pid: u32) -> Option<String> {
    read_link(pid, "cwd")
}

pub fn resolve_socket_observation(
    pid: u32,
    fd: u32,
) -> Result<Option<SocketEndpointObservation>, String> {
    let Some(inode) = read_socket_inode(pid, fd)? else {
        return Ok(None);
    };

    for (transport, table, ipv6) in [
        ("tcp", format!("/proc/{pid}/net/tcp"), false),
        ("tcp", format!("/proc/{pid}/net/tcp6"), true),
        ("udp", format!("/proc/{pid}/net/udp"), false),
        ("udp", format!("/proc/{pid}/net/udp6"), true),
    ] {
        if let Some(observation) = read_socket_table(&table, transport, ipv6, inode)? {
            return Ok(Some(observation));
        }
    }

    Ok(None)
}

fn read_socket_inode(pid: u32, fd: u32) -> Result<Option<u64>, String> {
    let path = format!("/proc/{pid}/fd/{fd}");
    let target = match std::fs::read_link(path) {
        Ok(target) => target.display().to_string(),
        Err(error) if proc_entry_gone(&error) => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let Some(raw_inode) = target
        .strip_prefix("socket:[")
        .and_then(|value| value.strip_suffix(']'))
    else {
        return Ok(None);
    };
    raw_inode
        .parse::<u64>()
        .map(Some)
        .map_err(|error| error.to_string())
}

fn read_socket_table(
    path: &str,
    transport: &str,
    ipv6: bool,
    inode: u64,
) -> Result<Option<SocketEndpointObservation>, String> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if proc_entry_gone(&error) => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };

    for line in raw.lines().skip(1) {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        let Some(entry_inode) = fields.get(9).and_then(|value| value.parse::<u64>().ok()) else {
            continue;
        };
        if entry_inode != inode {
            continue;
        }

        return Ok(Some(SocketEndpointObservation {
            transport: transport.to_string(),
            local: fields
                .get(1)
                .and_then(|value| parse_proc_endpoint(value, ipv6)),
            remote: fields
                .get(2)
                .and_then(|value| parse_proc_endpoint(value, ipv6)),
        }));
    }

    Ok(None)
}

fn parse_proc_endpoint(raw: &str, ipv6: bool) -> Option<String> {
    let (address, port) = raw.split_once(':')?;
    let port = u16::from_str_radix(port, 16).ok()?;
    if ipv6 {
        parse_ipv6_endpoint(address, port)
    } else {
        parse_ipv4_endpoint(address, port)
    }
}

fn parse_ipv4_endpoint(address: &str, port: u16) -> Option<String> {
    let bytes = hex_bytes(address)?;
    let ip = Ipv4Addr::new(bytes[3], bytes[2], bytes[1], bytes[0]);
    if ip.is_unspecified() && port == 0 {
        return None;
    }
    Some(format!("{ip}:{port}"))
}

fn parse_ipv6_endpoint(address: &str, port: u16) -> Option<String> {
    let raw = hex_bytes(address)?;
    let mut bytes = [0_u8; 16];
    for (chunk_index, chunk) in raw.chunks_exact(4).enumerate() {
        let target = chunk_index * 4;
        bytes[target] = chunk[3];
        bytes[target + 1] = chunk[2];
        bytes[target + 2] = chunk[1];
        bytes[target + 3] = chunk[0];
    }
    let ip = Ipv6Addr::from(bytes);
    if ip.is_unspecified() && port == 0 {
        return None;
    }
    Some(format!("[{ip}]:{port}"))
}

fn hex_bytes(raw: &str) -> Option<Vec<u8>> {
    if raw.len() % 2 != 0 {
        return None;
    }
    let mut bytes = Vec::with_capacity(raw.len() / 2);
    for chunk in raw.as_bytes().chunks_exact(2) {
        let pair = std::str::from_utf8(chunk).ok()?;
        bytes.push(u8::from_str_radix(pair, 16).ok()?);
    }
    Some(bytes)
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
