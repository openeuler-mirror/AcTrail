pub(super) mod decode;
pub(super) mod encode;
mod extra_codec;

use crate::storage::ResourceStorageError;
use rusqlite::{Connection, OptionalExtension};
use std::collections::BTreeMap;

// These codes are part of the on-disk format; new values must not reuse them.
const MEMORY_EVENTS: i64 = 1 << 6;
const MEMORY_EVENTS_LOCAL: i64 = 1 << 7;
const SOURCE_OVERRIDE: i64 = 1 << 8;

const HOST_COLUMNS: &str = "boot_key, observed_at_ns, cpu_percent_millis, cpu_count,
    memory_total_bytes, memory_available_bytes, memory_free_bytes, load_1, load_5, load_15";
const WORKLOAD_COLUMNS: &str = "trace_id, observed_at_ns, process_id, flags, alert,
    cpu_rate_interval_usec, cpu_percent_millis, rss_kb, virtual_memory_kb,
    memory_current_bytes, memory_peak_bytes, memory_anon_bytes, memory_file_bytes,
    memory_swap_current_bytes, cpu_usage_usec, cpu_user_usec, cpu_system_usec,
    cpu_nr_throttled, cpu_throttled_usec, io_read_bytes, io_write_bytes,
    pids_current, pids_peak, process_rss_sum_kb,
    mem_ev_low, mem_ev_high, mem_ev_max, mem_ev_oom, mem_ev_oom_kill, mem_ev_oom_group_kill,
    mem_ev_local_low, mem_ev_local_high, mem_ev_local_max, mem_ev_local_oom,
    mem_ev_local_oom_kill, mem_ev_local_oom_group_kill, extra";

struct TraceProfile {
    scope: String,
    subject: String,
    metadata: BTreeMap<String, String>,
}

impl TraceProfile {
    fn load(connection: &Connection, trace: i64) -> Result<Option<Self>, ResourceStorageError> {
        let raw = connection
            .prepare_cached(
                "SELECT scope, subject, metadata FROM resource_trace_profiles WHERE trace_id=?1",
            )?
            .query_row([trace], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get::<_, String>(2)?))
            })
            .optional()?;
        raw.map(|(scope, subject, metadata)| {
            Ok(Self {
                scope,
                subject,
                metadata: serde_json::from_str(&metadata)?,
            })
        })
        .transpose()
    }
}
