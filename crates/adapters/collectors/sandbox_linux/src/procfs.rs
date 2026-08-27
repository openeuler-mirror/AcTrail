//! Strict procfs parsing for process roots and Guest resources.

use std::collections::{HashMap, VecDeque};
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

pub(crate) struct ProcessCommSnapshot {
    pub(crate) pid: u32,
    pub(crate) executable_name: [u8; 16],
}

pub(crate) struct ProcessLineageSnapshot {
    pub(crate) root_count: usize,
    pub(crate) members: Vec<ProcessLineageMember>,
    pub(crate) process_comms: Vec<ProcessCommSnapshot>,
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
        let mut roots = processes
            .values()
            .filter(|process| names.contains(&process.marker.executable_name))
            .map(|process| process.marker)
            .collect::<Vec<_>>();
        roots.sort_unstable_by_key(|root| (root.pid, root.start_time_ticks));
        let mut children = HashMap::<u32, Vec<u32>>::new();
        for process in processes.values() {
            children
                .entry(process.parent_pid)
                .or_default()
                .push(process.marker.pid);
        }
        let mut assigned = HashMap::with_capacity(processes.len());
        let mut pending = VecDeque::with_capacity(processes.len());
        for root in &roots {
            assigned.insert(root.pid, *root);
            pending.push_back(root.pid);
        }
        while let Some(parent_pid) = pending.pop_front() {
            let root = assigned[&parent_pid];
            for child_pid in children.get(&parent_pid).into_iter().flatten() {
                if let std::collections::hash_map::Entry::Vacant(entry) = assigned.entry(*child_pid)
                {
                    entry.insert(root);
                    pending.push_back(*child_pid);
                }
            }
        }
        let mut members = assigned
            .into_iter()
            .map(|(pid, root)| ProcessLineageMember { pid, root })
            .collect::<Vec<_>>();
        members.sort_unstable_by_key(|member| member.pid);
        let mut process_comms = processes
            .values()
            .map(|process| ProcessCommSnapshot {
                pid: process.marker.pid,
                executable_name: process.marker.executable_name,
            })
            .collect::<Vec<_>>();
        process_comms.sort_unstable_by_key(|process| process.pid);
        Ok(ProcessLineageSnapshot {
            root_count: roots.len(),
            members,
            process_comms,
        })
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
        // guest and guest_nice are already included in user and nice by Linux;
        // only the first eight counters contribute to the non-duplicated total.
        let total_ticks = ticks.iter().take(8).try_fold(0_u64, |total, value| {
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

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use sandbox_observation::CpuSnapshot;

    use super::ProcfsReader;

    static NEXT_TEMP_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    struct TempProcfs {
        root: PathBuf,
    }

    impl TempProcfs {
        fn with_stat(stat: &str) -> Self {
            let sequence = NEXT_TEMP_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "actrail-sandbox-linux-procfs-{}-{sequence}",
                std::process::id()
            ));
            fs::create_dir(&root).expect("create temporary procfs root");
            fs::write(root.join("stat"), stat).expect("write temporary procfs stat");
            Self { root }
        }

        fn reader(&self) -> ProcfsReader {
            ProcfsReader::open(self.root.clone()).expect("open temporary procfs root")
        }
    }

    impl Drop for TempProcfs {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.root).expect("remove temporary procfs root");
        }
    }

    #[test]
    fn cpu_snapshot_uses_linux_cpu_counter_semantics() {
        let procfs = TempProcfs::with_stat(
            "cpu  10 20 30 40 5 6 7 8 900 1000\n\
             cpu0 1 2 3 4 5 6 7 8 9 10\n\
             cpu1 1 2 3 4 5 6 7 8 9 10\n\
             cpux 1 2 3 4 5 6 7 8 9 10\n\
             intr 42\n",
        );

        let snapshot = procfs.reader().cpu_snapshot().expect("read CPU snapshot");

        assert_eq!(
            snapshot,
            CpuSnapshot {
                total_ticks: 126,
                idle_ticks: 45,
                logical_cpu_count: 2,
            }
        );
    }

    #[test]
    fn cpu_snapshot_rejects_missing_aggregate_row() {
        let procfs = TempProcfs::with_stat("intr 42\ncpu0 1 2 3 4 5\n");

        let error = procfs.reader().cpu_snapshot().expect_err("reject stat");

        assert_eq!(error.stage(), "read_cpu");
        assert!(error.detail().contains("has no aggregate cpu row"));
    }

    #[test]
    fn cpu_snapshot_rejects_aggregate_with_too_few_counters() {
        let procfs = TempProcfs::with_stat("cpu 1 2 3 4\ncpu0 1 2 3 4 5\n");

        let error = procfs.reader().cpu_snapshot().expect_err("reject stat");

        assert_eq!(error.stage(), "read_cpu");
        assert!(
            error
                .detail()
                .contains("aggregate cpu row has fewer than five counters")
        );
    }

    #[test]
    fn cpu_snapshot_rejects_zero_logical_cpus() {
        let procfs = TempProcfs::with_stat(
            "cpu 1 2 3 4 5 6 7 8\n\
             cpux 1 2 3 4 5 6 7 8\n\
             intr 42\n",
        );

        let error = procfs.reader().cpu_snapshot().expect_err("reject stat");

        assert_eq!(error.stage(), "read_cpu");
        assert_eq!(error.detail(), "procfs reported zero logical CPUs");
    }

    #[test]
    fn cpu_snapshot_rejects_aggregate_counter_overflow() {
        let procfs =
            TempProcfs::with_stat(&format!("cpu {} 1 0 0 0 0 0 0\ncpu0 1 2 3 4 5\n", u64::MAX));

        let error = procfs.reader().cpu_snapshot().expect_err("reject stat");

        assert_eq!(error.stage(), "read_cpu");
        assert_eq!(error.detail(), "aggregate CPU tick counter overflow");
    }
}
