//! The few `/proc` facts checkpoint needs: parent, state and start time of each process.

use std::fs;
use std::io;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ProcStat {
    pub(super) pid: u32,
    pub(super) ppid: u32,
    /// The state letter of `/proc/<pid>/stat`, such as `R`, `S`, `T` or `Z`.
    pub(super) state: char,
    /// Field 20: a zombie leader can still have live threads in its group.
    pub(super) threads: u64,
    /// Field 22 of `/proc/<pid>/stat`, in clock ticks since boot.
    pub(super) start_time_ticks: u64,
}

impl ProcStat {
    pub(super) fn is_stopped(&self) -> bool {
        matches!(self.state, 'T' | 't')
    }

    pub(super) fn is_zombie(&self) -> bool {
        matches!(self.state, 'Z' | 'X')
    }

    pub(super) fn group_exited(&self) -> bool {
        self.is_zombie() && self.threads <= 1
    }
}

/// `None` when the process does not exist.
pub(super) fn read_stat(pid: u32) -> io::Result<Option<ProcStat>> {
    match fs::read(format!("/proc/{pid}/stat")) {
        Ok(raw) => parse_stat(pid, &raw).map(Some),
        Err(error) if process_gone(&error) => Ok(None),
        Err(error) => Err(error),
    }
}

/// Every live thread must be stopped, including ptrace stops; a leader can stop first.
pub(super) fn thread_group_stopped(pid: u32) -> io::Result<bool> {
    let threads = match fs::read_dir(format!("/proc/{pid}/task")) {
        Ok(threads) => threads,
        Err(error) if process_gone(&error) => {
            return Ok(read_stat(pid)?.is_none_or(|stat| stat.group_exited()));
        }
        Err(error) => return Err(error),
    };
    for thread in threads {
        let raw = match fs::read(thread?.path().join("stat")) {
            Ok(raw) => raw,
            Err(error) if process_gone(&error) => continue,
            Err(error) => return Err(error),
        };
        let stat = parse_stat(pid, &raw)?;
        if !stat.is_stopped() && !stat.is_zombie() {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Every process visible in `/proc`; processes that exit during the scan are skipped.
pub(super) fn all_processes() -> io::Result<Vec<ProcStat>> {
    let mut processes = Vec::new();
    for entry in fs::read_dir("/proc")? {
        let Some(pid) = entry?
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        if let Some(stat) = read_stat(pid)? {
            processes.push(stat);
        }
    }
    Ok(processes)
}

/// The target of a `/proc/<pid>/...` link, such as `ns/mnt` or `cwd`.
pub(super) fn read_link(pid: u32, name: &str) -> io::Result<String> {
    fs::read_link(format!("/proc/{pid}/{name}")).map(|target| target.to_string_lossy().into_owned())
}

fn process_gone(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::NotFound || error.raw_os_error() == Some(libc::ESRCH)
}

fn parse_stat(pid: u32, raw: &[u8]) -> io::Result<ProcStat> {
    // The command name may contain parentheses or end in a truncated UTF-8 character. Only
    // parse the ASCII fields after its final closing parenthesis.
    let fields: Vec<&[u8]> = raw
        .iter()
        .rposition(|byte| *byte == b')')
        .map(|end| {
            raw[end + 1..]
                .split(|byte| byte.is_ascii_whitespace())
                .filter(|field| !field.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let field = |index: usize| {
        fields.get(index).copied().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("/proc/{pid}/stat is too short"),
            )
        })
    };
    let number = |index: usize| -> io::Result<u64> {
        std::str::from_utf8(field(index)?)
            .ok()
            .and_then(|value| value.parse().ok())
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("/proc/{pid}/stat has a malformed field"),
                )
            })
    };
    Ok(ProcStat {
        pid,
        state: field(0)?.first().copied().map(char::from).unwrap_or('?'),
        threads: number(17)?,
        ppid: u32::try_from(number(1)?).unwrap_or_default(),
        start_time_ticks: number(19)?,
    })
}
