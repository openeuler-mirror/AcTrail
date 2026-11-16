use super::process_rss::CgroupProcessRss;
use linux_cgroup::CgroupCounters;

pub struct CgroupWorkloadSample {
    pub counters: CgroupCounters,
    pub cpu_percent_millis: Option<u64>,
    pub cpu_interval_usec: Option<u128>,
    pub process_rss: Option<CgroupProcessRss>,
    pub process_rss_error: Option<String>,
}
