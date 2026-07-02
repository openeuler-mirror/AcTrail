//! Procfs readers for resource metrics.

use std::fs;
use std::io;

use control_contract::reply::ControlError;

pub(super) const BYTES_PER_KIB: u64 = 1024;

const PROC_STAT_UTIME_INDEX: usize = 11;
const PROC_STAT_STIME_INDEX: usize = 12;
const PROC_STAT_THREADS_INDEX: usize = 17;
const PROC_STAT_MIN_FIELD_COUNT: usize = PROC_STAT_THREADS_INDEX + 1;

#[derive(Clone, Copy)]
pub(super) struct SystemUnits {
    pub clock_ticks_per_second: u64,
    pub page_size_kb: u64,
}

impl SystemUnits {
    pub(super) fn read() -> Result<Self, String> {
        let clock_ticks_per_second = sysconf_u64(libc::_SC_CLK_TCK, "clock ticks per second")?;
        let page_size_bytes = sysconf_u64(libc::_SC_PAGESIZE, "page size bytes")?;
        let page_size_kb = page_size_bytes / BYTES_PER_KIB;
        if page_size_kb == 0 {
            return Err(format!(
                "sysconf page size bytes returned {page_size_bytes}"
            ));
        }
        Ok(Self {
            clock_ticks_per_second,
            page_size_kb,
        })
    }
}

pub(super) struct ProcStat {
    pub comm: String,
    pub total_cpu_ticks: u64,
    pub threads: u64,
}

pub(super) struct ProcMemory {
    pub rss_kb: u64,
    pub virtual_memory_kb: u64,
}

pub(super) struct SystemMetrics {
    pub mem_total_kb: u64,
    pub mem_free_kb: u64,
    pub mem_available_kb: u64,
    pub loadavg_1m: String,
    pub loadavg_5m: String,
    pub loadavg_15m: String,
    pub loadavg_running_threads: String,
    pub loadavg_total_threads: String,
    pub loadavg_last_pid: String,
}

pub(super) fn read_proc_stat(pid: u32) -> Result<Option<ProcStat>, ControlError> {
    let Some(raw) = read_proc_file_optional(format!("/proc/{pid}/stat"))? else {
        return Ok(None);
    };
    parse_proc_stat(&raw).map(Some).map_err(|message| {
        ControlError::new(
            "resource_metrics_proc_stat",
            format!("pid={pid}: {message}"),
        )
    })
}

pub(super) fn read_proc_memory(
    pid: u32,
    page_size_kb: u64,
) -> Result<Option<ProcMemory>, ControlError> {
    let Some(raw) = read_proc_file_optional(format!("/proc/{pid}/statm"))? else {
        return Ok(None);
    };
    let fields = raw.split_whitespace().collect::<Vec<_>>();
    if fields.len() < 2 {
        return Err(ControlError::new(
            "resource_metrics_proc_statm",
            format!("pid={pid}: not enough statm fields"),
        ));
    }
    let virtual_pages = parse_u64(fields[0], "virtual pages")
        .map_err(|message| ControlError::new("resource_metrics_proc_statm", message))?;
    let rss_pages = parse_u64(fields[1], "rss pages")
        .map_err(|message| ControlError::new("resource_metrics_proc_statm", message))?;
    Ok(Some(ProcMemory {
        rss_kb: rss_pages.saturating_mul(page_size_kb),
        virtual_memory_kb: virtual_pages.saturating_mul(page_size_kb),
    }))
}

pub(super) fn cpu_cores() -> Result<String, ControlError> {
    std::thread::available_parallelism()
        .map(|value| value.get().to_string())
        .map_err(|error| ControlError::new("resource_metrics_cpu_cores", error.to_string()))
}

pub(super) fn read_system_metrics() -> Result<SystemMetrics, ControlError> {
    let meminfo = fs::read_to_string("/proc/meminfo").map_err(|error| {
        ControlError::new(
            "resource_metrics_procfs",
            format!("read /proc/meminfo: {error}"),
        )
    })?;
    let loadavg = fs::read_to_string("/proc/loadavg").map_err(|error| {
        ControlError::new(
            "resource_metrics_procfs",
            format!("read /proc/loadavg: {error}"),
        )
    })?;
    let (loadavg_running_threads, loadavg_total_threads, loadavg_last_pid) =
        parse_loadavg_threads(&loadavg)?;
    Ok(SystemMetrics {
        mem_total_kb: required_meminfo_kb(&meminfo, "MemTotal")?,
        mem_free_kb: required_meminfo_kb(&meminfo, "MemFree")?,
        mem_available_kb: required_meminfo_kb(&meminfo, "MemAvailable")?,
        loadavg_1m: required_loadavg_field(&loadavg, 0, "1m")?,
        loadavg_5m: required_loadavg_field(&loadavg, 1, "5m")?,
        loadavg_15m: required_loadavg_field(&loadavg, 2, "15m")?,
        loadavg_running_threads,
        loadavg_total_threads,
        loadavg_last_pid,
    })
}

fn parse_proc_stat(raw: &str) -> Result<ProcStat, String> {
    let open = raw
        .find('(')
        .ok_or_else(|| "missing comm open delimiter".to_string())?;
    let close = raw
        .rfind(')')
        .ok_or_else(|| "missing comm close delimiter".to_string())?;
    if close <= open {
        return Err("invalid comm delimiters".to_string());
    }
    let comm = raw[open + 1..close].to_string();
    let fields = raw[close + 1..].split_whitespace().collect::<Vec<_>>();
    if fields.len() < PROC_STAT_MIN_FIELD_COUNT {
        return Err("not enough stat fields".to_string());
    }
    let utime = parse_u64(fields[PROC_STAT_UTIME_INDEX], "utime")?;
    let stime = parse_u64(fields[PROC_STAT_STIME_INDEX], "stime")?;
    let threads = parse_u64(fields[PROC_STAT_THREADS_INDEX], "num_threads")?;
    Ok(ProcStat {
        comm,
        total_cpu_ticks: utime.saturating_add(stime),
        threads,
    })
}

fn read_proc_file_optional(path: String) -> Result<Option<String>, ControlError> {
    match fs::read_to_string(&path) {
        Ok(value) => Ok(Some(value)),
        Err(error) if proc_entry_gone(&error) => Ok(None),
        Err(error) => Err(ControlError::new(
            "resource_metrics_procfs",
            format!("read {path}: {error}"),
        )),
    }
}

fn proc_entry_gone(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::NotFound || error.raw_os_error() == Some(libc::ESRCH)
}

fn parse_u64(raw: &str, field: &str) -> Result<u64, String> {
    raw.parse::<u64>()
        .map_err(|error| format!("invalid {field}: {error}"))
}

fn required_meminfo_kb(raw: &str, key: &str) -> Result<u64, ControlError> {
    for line in raw.lines() {
        let Some((name, rest)) = line.split_once(':') else {
            continue;
        };
        if name != key {
            continue;
        }
        let value = rest.split_whitespace().next().ok_or_else(|| {
            ControlError::new(
                "resource_metrics_proc_meminfo",
                format!("{key} missing value"),
            )
        })?;
        return parse_u64(value, key)
            .map_err(|message| ControlError::new("resource_metrics_proc_meminfo", message));
    }
    Err(ControlError::new(
        "resource_metrics_proc_meminfo",
        format!("missing {key}"),
    ))
}

fn required_loadavg_field(raw: &str, index: usize, label: &str) -> Result<String, ControlError> {
    raw.split_whitespace()
        .nth(index)
        .map(str::to_string)
        .ok_or_else(|| {
            ControlError::new(
                "resource_metrics_proc_loadavg",
                format!("missing loadavg {label}"),
            )
        })
}

fn parse_loadavg_threads(raw: &str) -> Result<(String, String, String), ControlError> {
    let threads = required_loadavg_field(raw, 3, "threads")?;
    let Some((running, total)) = threads.split_once('/') else {
        return Err(ControlError::new(
            "resource_metrics_proc_loadavg",
            "invalid loadavg thread field",
        ));
    };
    Ok((
        running.to_string(),
        total.to_string(),
        required_loadavg_field(raw, 4, "last_pid")?,
    ))
}

fn sysconf_u64(name: libc::c_int, label: &str) -> Result<u64, String> {
    let value = unsafe { libc::sysconf(name) };
    if value <= 0 {
        return Err(format!("sysconf {label} returned {value}"));
    }
    u64::try_from(value).map_err(|error| format!("sysconf {label}: {error}"))
}
