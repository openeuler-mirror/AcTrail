//! Strict procfs parsing for process roots and Guest resources.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use sandbox_observation::{CpuSnapshot, GuestBootId, MemorySnapshot, ProcessMarker};

use crate::SandboxLinuxError;

pub(crate) struct ProcfsReader {
    root: PathBuf,
}

pub(crate) struct ProcessLineageMember {
    pub(crate) pid: u32,
    pub(crate) root: ProcessMarker,
}

pub(crate) struct ProcessLineageSnapshot {
    pub(crate) root_count: usize,
    pub(crate) members: Vec<ProcessLineageMember>,
}

struct ProcessSnapshot {
    marker: ProcessMarker,
    parent_pid: u32,
}

impl ProcfsReader {
    pub(crate) fn open(root: PathBuf) -> Result<Self, SandboxLinuxError> {
        let metadata = fs::metadata(&root).map_err(|error| {
            SandboxLinuxError::new(
                "open_procfs",
                format!("cannot inspect {}: {error}", root.display()),
            )
        })?;
        if !metadata.is_dir() {
            return Err(SandboxLinuxError::new(
                "open_procfs",
                format!("{} is not a directory", root.display()),
            ));
        }
        Ok(Self { root })
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn boot_id(&self) -> Result<GuestBootId, SandboxLinuxError> {
        let path = self.root.join("sys/kernel/random/boot_id");
        let raw = fs::read_to_string(&path).map_err(|error| {
            SandboxLinuxError::new(
                "read_boot_id",
                format!("cannot read {}: {error}", path.display()),
            )
        })?;
        let compact = raw.trim().replace('-', "");
        if compact.len() != 32 {
            return Err(SandboxLinuxError::new(
                "read_boot_id",
                format!("{} contains an invalid boot id", path.display()),
            ));
        }
        let mut bytes = [0_u8; 16];
        for (index, slot) in bytes.iter_mut().enumerate() {
            let offset = index * 2;
            *slot = u8::from_str_radix(&compact[offset..offset + 2], 16).map_err(|error| {
                SandboxLinuxError::new(
                    "read_boot_id",
                    format!("{} contains an invalid boot id: {error}", path.display()),
                )
            })?;
        }
        Ok(GuestBootId::new(bytes))
    }

    pub(crate) fn discover_lineages(
        &self,
        names: &[[u8; 16]],
    ) -> Result<ProcessLineageSnapshot, SandboxLinuxError> {
        let entries = fs::read_dir(&self.root).map_err(|error| {
            SandboxLinuxError::new(
                "discover_lineages",
                format!("cannot enumerate {}: {error}", self.root.display()),
            )
        })?;
        let mut processes = HashMap::new();
        for entry in entries {
            let Ok(entry) = entry else { continue };
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|name| name.parse::<u32>().ok())
            else {
                continue;
            };
            match self.process_snapshot(pid) {
                Ok(snapshot) => {
                    processes.insert(pid, snapshot);
                }
                Err(error) if is_transient_process_error(&error) => {}
                Err(_) => {}
            }
        }
        let roots = processes
            .values()
            .filter(|process| names.contains(&process.marker.executable_name))
            .map(|process| (process.marker.pid, process.marker))
            .collect::<HashMap<_, _>>();
        let mut members = processes
            .keys()
            .filter_map(|pid| {
                Self::nearest_root(*pid, &processes, &roots)
                    .map(|root| ProcessLineageMember { pid: *pid, root })
            })
            .collect::<Vec<_>>();
        members.sort_unstable_by_key(|member| member.pid);
        Ok(ProcessLineageSnapshot {
            root_count: roots.len(),
            members,
        })
    }

    fn nearest_root(
        start_pid: u32,
        processes: &HashMap<u32, ProcessSnapshot>,
        roots: &HashMap<u32, ProcessMarker>,
    ) -> Option<ProcessMarker> {
        let mut current = start_pid;
        let mut remaining = processes.len();
        while current != 0 && remaining > 0 {
            if let Some(root) = roots.get(&current) {
                return Some(*root);
            }
            current = processes.get(&current)?.parent_pid;
            remaining -= 1;
        }
        None
    }

    pub(crate) fn cpu_snapshot(&self) -> Result<CpuSnapshot, SandboxLinuxError> {
        let path = self.root.join("stat");
        let raw = fs::read_to_string(&path).map_err(|error| {
            SandboxLinuxError::new(
                "read_cpu",
                format!("cannot read {}: {error}", path.display()),
            )
        })?;
        let mut lines = raw.lines();
        let aggregate = lines.next().ok_or_else(|| {
            SandboxLinuxError::new("read_cpu", format!("{} is empty", path.display()))
        })?;
        let mut fields = aggregate.split_ascii_whitespace();
        if fields.next() != Some("cpu") {
            return Err(SandboxLinuxError::new(
                "read_cpu",
                format!("{} has no aggregate cpu row", path.display()),
            ));
        }
        let ticks = fields
            .map(|value| value.parse::<u64>())
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| SandboxLinuxError::new("read_cpu", error.to_string()))?;
        if ticks.len() < 5 {
            return Err(SandboxLinuxError::new(
                "read_cpu",
                format!(
                    "{} aggregate cpu row has fewer than five counters",
                    path.display()
                ),
            ));
        }
        let total_ticks = ticks.iter().try_fold(0_u64, |total, value| {
            total.checked_add(*value).ok_or_else(|| {
                SandboxLinuxError::new("read_cpu", "aggregate CPU tick counter overflow")
            })
        })?;
        let idle_ticks = ticks[3]
            .checked_add(ticks[4])
            .ok_or_else(|| SandboxLinuxError::new("read_cpu", "idle CPU tick counter overflow"))?;
        let logical_cpu_count = lines
            .filter_map(|line| line.split_ascii_whitespace().next())
            .filter(|label| {
                label.strip_prefix("cpu").is_some_and(|suffix| {
                    !suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_digit())
                })
            })
            .count();
        let logical_cpu_count = u16::try_from(logical_cpu_count).map_err(|error| {
            SandboxLinuxError::new("read_cpu", format!("logical CPU count overflow: {error}"))
        })?;
        if logical_cpu_count == 0 {
            return Err(SandboxLinuxError::new(
                "read_cpu",
                "procfs reported zero logical CPUs",
            ));
        }
        Ok(CpuSnapshot {
            total_ticks,
            idle_ticks,
            logical_cpu_count,
        })
    }

    pub(crate) fn memory_snapshot(&self) -> Result<MemorySnapshot, SandboxLinuxError> {
        let path = self.root.join("meminfo");
        let raw = fs::read_to_string(&path).map_err(|error| {
            SandboxLinuxError::new(
                "read_memory",
                format!("cannot read {}: {error}", path.display()),
            )
        })?;
        let total_bytes = Self::meminfo_bytes(&raw, "MemTotal")?;
        let available_bytes = Self::meminfo_bytes(&raw, "MemAvailable")?;
        let used_bytes = total_bytes.checked_sub(available_bytes).ok_or_else(|| {
            SandboxLinuxError::new("read_memory", "MemAvailable exceeds MemTotal")
        })?;
        let oom_kill_count = self.oom_kill_count()?;
        Ok(MemorySnapshot {
            total_bytes,
            available_bytes,
            used_bytes,
            oom_kill_count,
        })
    }

    fn oom_kill_count(&self) -> Result<u64, SandboxLinuxError> {
        let path = self.root.join("vmstat");
        let raw = fs::read_to_string(&path).map_err(|error| {
            SandboxLinuxError::new(
                "read_oom_kill",
                format!("cannot read {}: {error}", path.display()),
            )
        })?;
        let mut value = None;
        for line in raw.lines() {
            let mut fields = line.split_ascii_whitespace();
            if fields.next() != Some("oom_kill") {
                continue;
            }
            if value.is_some() {
                return Err(SandboxLinuxError::new(
                    "read_oom_kill",
                    format!("{} contains duplicate oom_kill counters", path.display()),
                ));
            }
            let count = fields
                .next()
                .ok_or_else(|| {
                    SandboxLinuxError::new(
                        "read_oom_kill",
                        format!("{} oom_kill row has no value", path.display()),
                    )
                })?
                .parse::<u64>()
                .map_err(|error| {
                    SandboxLinuxError::new(
                        "read_oom_kill",
                        format!("{} has invalid oom_kill value: {error}", path.display()),
                    )
                })?;
            if fields.next().is_some() {
                return Err(SandboxLinuxError::new(
                    "read_oom_kill",
                    format!("{} oom_kill row has trailing fields", path.display()),
                ));
            }
            value = Some(count);
        }
        value.ok_or_else(|| {
            SandboxLinuxError::new(
                "read_oom_kill",
                format!("{} is missing oom_kill", path.display()),
            )
        })
    }

    fn process_snapshot(&self, pid: u32) -> io::Result<ProcessSnapshot> {
        let directory = self.root.join(pid.to_string());
        let comm = fs::read(directory.join("comm"))?;
        let comm = comm.strip_suffix(b"\n").unwrap_or(&comm);
        if comm.is_empty() || comm.len() > 15 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "process comm is outside the Linux TASK_COMM_LEN bound",
            ));
        }
        let stat = fs::read_to_string(directory.join("stat"))?;
        let close = stat.rfind(')').ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "process stat has no closing comm",
            )
        })?;
        let mut fields = stat[close + 1..].split_ascii_whitespace();
        let parent_pid = fields
            .nth(1)
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "process stat has no parent pid")
            })?
            .parse::<u32>()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let start_time_ticks = fields
            .nth(17)
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "process stat has no starttime")
            })?
            .parse::<u64>()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let mut executable_name = [0_u8; 16];
        executable_name[..comm.len()].copy_from_slice(comm);
        Ok(ProcessSnapshot {
            marker: ProcessMarker {
                pid,
                start_time_ticks,
                executable_name,
            },
            parent_pid,
        })
    }

    fn meminfo_bytes(raw: &str, key: &str) -> Result<u64, SandboxLinuxError> {
        let line = raw
            .lines()
            .find(|line| line.starts_with(key) && line.as_bytes().get(key.len()) == Some(&b':'))
            .ok_or_else(|| {
                SandboxLinuxError::new("read_memory", format!("meminfo is missing {key}"))
            })?;
        let mut fields = line[key.len() + 1..].split_ascii_whitespace();
        let kib = fields
            .next()
            .ok_or_else(|| SandboxLinuxError::new("read_memory", format!("{key} has no value")))?
            .parse::<u64>()
            .map_err(|error| SandboxLinuxError::new("read_memory", error.to_string()))?;
        if fields.next() != Some("kB") || fields.next().is_some() {
            return Err(SandboxLinuxError::new(
                "read_memory",
                format!("{key} must contain one kB value"),
            ));
        }
        kib.checked_mul(1024).ok_or_else(|| {
            SandboxLinuxError::new("read_memory", format!("{key} byte count overflow"))
        })
    }
}

fn is_transient_process_error(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::NotFound
            | io::ErrorKind::PermissionDenied
            | io::ErrorKind::InvalidData
            | io::ErrorKind::UnexpectedEof
    )
}
