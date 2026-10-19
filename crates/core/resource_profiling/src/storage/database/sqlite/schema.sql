CREATE TABLE IF NOT EXISTS resource_samples (
    id INTEGER PRIMARY KEY,
    kind TEXT NOT NULL CHECK(kind IN ('host','workload')),
    observed_at_ns INTEGER NOT NULL,
    host_boot_id TEXT,
    trace_id INTEGER,
    payload TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS resource_time ON resource_samples(observed_at_ns);
CREATE INDEX IF NOT EXISTS resource_host ON resource_samples(host_boot_id,observed_at_ns);
CREATE INDEX IF NOT EXISTS resource_trace ON resource_samples(trace_id,observed_at_ns);
