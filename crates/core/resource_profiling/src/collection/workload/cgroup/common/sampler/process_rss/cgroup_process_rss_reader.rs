use std::{collections::BTreeSet, fs, io};

use super::super::CgroupCounterSource;
use super::CgroupProcessRssConfig;

#[derive(Clone, Copy, Debug)]
pub struct CgroupProcessRss {
    pub rss_sum_kb: u64,
    pub candidate_processes: usize,
    pub sampled_processes: usize,
}

pub(in crate::collection::workload::cgroup) struct CgroupProcessRssReader {
    page_size_kb: Option<u64>,
}

impl CgroupProcessRssReader {
    pub fn new(config: CgroupProcessRssConfig) -> Result<Self, String> {
        let page_size_kb = if config.enabled {
            let bytes = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
            if bytes < 1024 {
                return Err(format!("invalid system page size: {bytes}"));
            }
            Some(bytes as u64 / 1024)
        } else {
            None
        };
        Ok(Self { page_size_kb })
    }

    pub fn read(
        &self,
        source: &impl CgroupCounterSource,
    ) -> Result<Option<CgroupProcessRss>, String> {
        let Some(page_size_kb) = self.page_size_kb else {
            return Ok(None);
        };
        let candidates = source
            .read_subtree_pids()
            .map_err(|error| error.to_string())?;
        let mut samples = Vec::with_capacity(candidates.len());
        for pid in &candidates {
            let Some(before) = Self::read_file(*pid, "stat")? else {
                continue;
            };
            let Some(memory) = Self::read_file(*pid, "statm")? else {
                continue;
            };
            let Some(after) = Self::read_file(*pid, "stat")? else {
                continue;
            };
            if Self::start_ticks(&before)? != Self::start_ticks(&after)? {
                continue;
            }
            let pages = memory
                .split_whitespace()
                .nth(1)
                .ok_or("statm lacks resident pages")?
                .parse::<u64>()
                .map_err(|error| error.to_string())?;
            samples.push((*pid, pages.saturating_mul(page_size_kb)));
        }
        let current: BTreeSet<_> = source
            .read_subtree_pids()
            .map_err(|error| error.to_string())?
            .into_iter()
            .collect();
        let mut result = CgroupProcessRss {
            rss_sum_kb: 0,
            candidate_processes: candidates.len(),
            sampled_processes: 0,
        };
        for (pid, rss) in samples {
            if current.contains(&pid) {
                result.rss_sum_kb = result.rss_sum_kb.saturating_add(rss);
                result.sampled_processes += 1;
            }
        }
        Ok(Some(result))
    }

    fn read_file(pid: u32, name: &str) -> Result<Option<String>, String> {
        match fs::read_to_string(format!("/proc/{pid}/{name}")) {
            Ok(raw) => Ok(Some(raw)),
            Err(error)
                if error.kind() == io::ErrorKind::NotFound
                    || error.raw_os_error() == Some(libc::ESRCH) =>
            {
                Ok(None)
            }
            Err(error) => Err(format!("read /proc/{pid}/{name}: {error}")),
        }
    }

    fn start_ticks(stat: &str) -> Result<u64, String> {
        let end = stat.rfind(')').ok_or("stat lacks command terminator")?;
        stat[end + 1..]
            .split_whitespace()
            .nth(19)
            .ok_or("stat lacks start time")?
            .parse()
            .map_err(|error| format!("invalid stat start time: {error}"))
    }
}
