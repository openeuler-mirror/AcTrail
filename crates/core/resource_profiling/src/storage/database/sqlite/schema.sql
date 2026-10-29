DROP TABLE IF EXISTS resource_samples;

CREATE TABLE IF NOT EXISTS resource_host_boots (
    boot_key INTEGER PRIMARY KEY,
    boot_id BLOB NOT NULL UNIQUE CHECK (length(boot_id) = 16)
);
CREATE TABLE IF NOT EXISTS resource_host_samples (
    id INTEGER PRIMARY KEY,
    boot_key INTEGER NOT NULL,
    observed_at_ns INTEGER NOT NULL,
    cpu_percent_millis INTEGER,
    cpu_count INTEGER,
    memory_total_bytes INTEGER NOT NULL,
    memory_available_bytes INTEGER NOT NULL,
    memory_free_bytes INTEGER NOT NULL,
    load_1 REAL NOT NULL,
    load_5 REAL NOT NULL,
    load_15 REAL NOT NULL
);
CREATE INDEX IF NOT EXISTS resource_host_samples_boot ON resource_host_samples(boot_key, observed_at_ns);
CREATE INDEX IF NOT EXISTS resource_host_samples_age ON resource_host_samples(observed_at_ns);

-- Immutable: every extra in this trace is relative to its first sample.
CREATE TABLE IF NOT EXISTS resource_trace_profiles (
    trace_id INTEGER PRIMARY KEY,
    scope TEXT NOT NULL,
    subject TEXT NOT NULL,
    metadata TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS resource_metadata_keys (
    key_id INTEGER PRIMARY KEY,
    key TEXT NOT NULL UNIQUE
);
CREATE TABLE IF NOT EXISTS resource_workload_samples (
    id INTEGER PRIMARY KEY,
    trace_id INTEGER NOT NULL,
    observed_at_ns INTEGER NOT NULL,
    process_id INTEGER NOT NULL,
    flags INTEGER NOT NULL,
    alert INTEGER,
    cpu_rate_interval_usec INTEGER,
    cpu_percent_millis INTEGER,
    rss_kb INTEGER,
    virtual_memory_kb INTEGER,
    memory_current_bytes INTEGER,
    memory_peak_bytes INTEGER,
    memory_anon_bytes INTEGER,
    memory_file_bytes INTEGER,
    memory_swap_current_bytes INTEGER,
    cpu_usage_usec INTEGER,
    cpu_user_usec INTEGER,
    cpu_system_usec INTEGER,
    cpu_nr_throttled INTEGER,
    cpu_throttled_usec INTEGER,
    io_read_bytes INTEGER,
    io_write_bytes INTEGER,
    pids_current INTEGER,
    pids_peak INTEGER,
    process_rss_sum_kb INTEGER,
    mem_ev_low INTEGER,
    mem_ev_high INTEGER,
    mem_ev_max INTEGER,
    mem_ev_oom INTEGER,
    mem_ev_oom_kill INTEGER,
    mem_ev_oom_group_kill INTEGER,
    mem_ev_local_low INTEGER,
    mem_ev_local_high INTEGER,
    mem_ev_local_max INTEGER,
    mem_ev_local_oom INTEGER,
    mem_ev_local_oom_kill INTEGER,
    mem_ev_local_oom_group_kill INTEGER,
    extra BLOB
);
CREATE INDEX IF NOT EXISTS resource_workload_samples_trace ON resource_workload_samples(trace_id, observed_at_ns);
CREATE INDEX IF NOT EXISTS resource_workload_samples_age ON resource_workload_samples(observed_at_ns);
