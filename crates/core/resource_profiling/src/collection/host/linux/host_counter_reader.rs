use std::fs;

pub(super) struct HostCounterReader;

pub(super) struct HostCounters {
    pub busy_ticks: u64,
    pub total_ticks: u64,
    pub memory_total_bytes: u64,
    pub memory_available_bytes: u64,
    pub memory_free_bytes: u64,
    pub load_average: [f64; 3],
    pub cpu_count: u32,
}

impl HostCounterReader {
    pub fn read(&self) -> Result<HostCounters, String> {
        let stat = fs::read_to_string("/proc/stat").map_err(|error| error.to_string())?;
        let line = stat
            .lines()
            .find(|line| line.starts_with("cpu "))
            .ok_or("/proc/stat: missing aggregate CPU counters")?;
        // guest and guest_nice are already included in user and nice.
        let ticks = line
            .split_whitespace()
            .skip(1)
            .take(8)
            .map(|value| value.parse::<u64>().map_err(|error| error.to_string()))
            .collect::<Result<Vec<_>, _>>()?;
        if ticks.len() < 4 {
            return Err("/proc/stat: incomplete CPU counters".into());
        }
        let total_ticks = ticks
            .iter()
            .try_fold(0_u64, |sum, value| sum.checked_add(*value))
            .ok_or("/proc/stat: CPU counter overflow")?;
        let idle = ticks[3]
            .checked_add(ticks.get(4).copied().unwrap_or(0))
            .ok_or("/proc/stat: idle counter overflow")?;
        let cpu_count = Self::cpu_count(&stat);
        let meminfo = fs::read_to_string("/proc/meminfo").map_err(|error| error.to_string())?;
        let loadavg = fs::read_to_string("/proc/loadavg").map_err(|error| error.to_string())?;
        let mut load = loadavg.split_whitespace();
        let mut load_average = [0.0; 3];
        for value in &mut load_average {
            *value = load
                .next()
                .ok_or("/proc/loadavg: missing load")?
                .parse::<f64>()
                .map_err(|error| error.to_string())?;
            if !value.is_finite() || *value < 0.0 {
                return Err("/proc/loadavg: invalid load".into());
            }
        }
        Ok(HostCounters {
            busy_ticks: total_ticks - idle,
            total_ticks,
            memory_total_bytes: Self::memory(&meminfo, "MemTotal:")?,
            memory_available_bytes: Self::memory(&meminfo, "MemAvailable:")?,
            memory_free_bytes: Self::memory(&meminfo, "MemFree:")?,
            load_average,
            cpu_count,
        })
    }

    /// The per-CPU lines of /proc/stat, which list exactly the online CPUs.
    pub fn cpu_count(stat: &str) -> u32 {
        stat.lines()
            .filter(|line| {
                line.strip_prefix("cpu")
                    .is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit()))
            })
            .count() as u32
    }

    fn memory(meminfo: &str, key: &str) -> Result<u64, String> {
        let mut fields = meminfo
            .lines()
            .find(|line| line.starts_with(key))
            .ok_or_else(|| format!("/proc/meminfo: missing {key}"))?
            .split_whitespace()
            .skip(1);
        let value = fields
            .next()
            .ok_or("/proc/meminfo: missing value")?
            .parse::<u64>()
            .map_err(|error| error.to_string())?;
        if fields.next() != Some("kB") {
            return Err("/proc/meminfo: expected kB".into());
        }
        value
            .checked_mul(1024)
            .ok_or_else(|| "/proc/meminfo: counter overflow".into())
    }
}
