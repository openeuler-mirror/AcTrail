CREATE TABLE IF NOT EXISTS trace_external_cgroup_v1_bindings (
    trace_id INTEGER PRIMARY KEY REFERENCES traces(trace_id),
    host_boot_id BLOB NOT NULL CHECK(length(host_boot_id)=16),
    cpuacct TEXT NOT NULL,
    memory TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    stale_reason TEXT,
    closed_at INTEGER
);
CREATE INDEX IF NOT EXISTS cgroup_v1_live ON trace_external_cgroup_v1_bindings(closed_at);
