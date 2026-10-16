/// Process-tree counters. CPU is absent until every sampled process has a baseline.
pub struct ProcfsWorkloadSample {
    pub cpu_percent_millis: Option<u64>,
    pub rss_kb: u64,
    pub virtual_memory_kb: u64,
    pub sampled_processes: usize,
    pub root_threads: Option<u64>,
    pub root_comm: Option<String>,
}
