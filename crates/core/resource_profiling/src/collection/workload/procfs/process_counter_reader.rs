use std::{fs, io};

use super::VerifiedProcessTarget;

const BYTES_PER_KIB: u64 = 1024;
const UTIME: usize = 11;
const STIME: usize = 12;
const THREADS: usize = 17;
const START_TIME: usize = 19;

pub(super) struct ProcessCounterReader {
    page_size_kb: u64,
    clock_ticks_per_second: u64,
}

pub(super) struct ProcessCounters {
    pub cpu_ticks: u64,
    pub rss_kb: u64,
    pub virtual_memory_kb: u64,
    pub threads: u64,
    pub comm: String,
}

struct ProcessStat {
    exited: bool,
    cpu_ticks: u64,
    start_time_ticks: u64,
    threads: u64,
    comm: String,
}

impl ProcessCounterReader {
    pub fn new() -> Result<Self, String> {
        let page_size_kb = Self::system_unit(libc::_SC_PAGESIZE)? / BYTES_PER_KIB;
        if page_size_kb == 0 {
            return Err("procfs page size is smaller than one KiB".to_string());
        }
        Ok(Self {
            page_size_kb,
            clock_ticks_per_second: Self::system_unit(libc::_SC_CLK_TCK)?,
        })
    }

    pub fn clock_ticks_per_second(&self) -> u64 {
        self.clock_ticks_per_second
    }

    pub fn read(&self, target: &VerifiedProcessTarget) -> Result<Option<ProcessCounters>, String> {
        let Some(before) = Self::read_stat(target.observer_pid)? else {
            return Ok(None);
        };
        if before.exited || before.start_time_ticks != target.expected_start_time_ticks {
            return Ok(None);
        }
        let Some(memory) = Self::read_optional(target.observer_pid, "statm")? else {
            return Ok(None);
        };
        let mut fields = memory.split_whitespace();
        let virtual_pages = Self::number(fields.next(), "virtual pages")?;
        let rss_pages = Self::number(fields.next(), "RSS pages")?;
        // stat and statm are separate reads; never charge a reused PID to its predecessor.
        let Some(after) = Self::read_stat(target.observer_pid)? else {
            return Ok(None);
        };
        if after.exited || after.start_time_ticks != target.expected_start_time_ticks {
            return Ok(None);
        }
        Ok(Some(ProcessCounters {
            cpu_ticks: before.cpu_ticks,
            rss_kb: rss_pages.saturating_mul(self.page_size_kb),
            virtual_memory_kb: virtual_pages.saturating_mul(self.page_size_kb),
            threads: before.threads,
            comm: before.comm,
        }))
    }

    pub(super) fn is_gone(target: &VerifiedProcessTarget) -> Result<bool, String> {
        Ok(Self::read_stat(target.observer_pid)?.is_none_or(|stat| {
            stat.exited || stat.start_time_ticks != target.expected_start_time_ticks
        }))
    }

    fn read_stat(pid: u32) -> Result<Option<ProcessStat>, String> {
        let Some(raw) = Self::read_optional(pid, "stat")? else {
            return Ok(None);
        };
        let open = raw
            .find('(')
            .ok_or("proc stat missing comm open delimiter")?;
        let close = raw
            .rfind(')')
            .ok_or("proc stat missing comm close delimiter")?;
        if close <= open {
            return Err("proc stat has invalid comm delimiters".to_string());
        }
        let fields: Vec<_> = raw[close + 1..].split_whitespace().collect();
        let value = |index: usize, name| Self::number(fields.get(index).copied(), name);
        Ok(Some(ProcessStat {
            exited: matches!(fields.first().copied(), Some("Z" | "X" | "x")),
            cpu_ticks: value(UTIME, "utime")?.saturating_add(value(STIME, "stime")?),
            start_time_ticks: value(START_TIME, "starttime")?,
            threads: value(THREADS, "num_threads")?,
            comm: raw[open + 1..close].to_string(),
        }))
    }

    fn read_optional(pid: u32, name: &str) -> Result<Option<String>, String> {
        let path = format!("/proc/{pid}/{name}");
        match fs::read_to_string(&path) {
            Ok(raw) => Ok(Some(raw)),
            Err(error)
                if error.kind() == io::ErrorKind::NotFound
                    || error.raw_os_error() == Some(libc::ESRCH) =>
            {
                Ok(None)
            }
            Err(error) => Err(format!("read {path}: {error}")),
        }
    }

    fn number(raw: Option<&str>, field: &str) -> Result<u64, String> {
        raw.ok_or_else(|| format!("procfs missing {field}"))?
            .parse::<u64>()
            .map_err(|error| format!("invalid procfs {field}: {error}"))
    }

    fn system_unit(name: libc::c_int) -> Result<u64, String> {
        let value = unsafe { libc::sysconf(name) };
        if value <= 0 {
            return Err(format!("sysconf({name}) returned {value}"));
        }
        u64::try_from(value).map_err(|error| error.to_string())
    }
}
