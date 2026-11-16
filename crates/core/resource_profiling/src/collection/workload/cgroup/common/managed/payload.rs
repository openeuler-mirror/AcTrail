use crate::model::{MemoryEventCounters, ResourceMetrics};

fn convert_memory_events(events: linux_cgroup::MemoryEvents) -> MemoryEventCounters {
    MemoryEventCounters {
        low: events.low,
        high: events.high,
        max: events.max,
        oom: events.oom,
        oom_kill: events.oom_kill,
        oom_group_kill: events.oom_group_kill,
    }
}

/// Copies cgroup counters into a sample payload (managed scopes and bindings).
pub(crate) fn apply_cgroup_counters(
    payload: &mut ResourceMetrics,
    counters: linux_cgroup::CgroupCounters,
) {
    payload.memory_current_bytes = Some(counters.memory_current_bytes);
    payload.memory_peak_bytes = counters.memory_peak_bytes;
    payload.memory_anon_bytes = counters.memory_anon_bytes;
    payload.memory_file_bytes = counters.memory_file_bytes;
    payload.memory_swap_current_bytes = counters.memory_swap_current_bytes;
    payload.memory_events = counters.memory_events.map(convert_memory_events);
    payload.memory_events_local = counters.memory_events_local.map(convert_memory_events);
    payload.cpu_usage_usec = Some(counters.cpu.usage_usec);
    payload.cpu_user_usec = counters.cpu.user_usec;
    payload.cpu_system_usec = counters.cpu.system_usec;
    payload.cpu_nr_throttled = counters.cpu.nr_throttled;
    payload.cpu_throttled_usec = counters.cpu.throttled_usec;
    payload.io_read_bytes = counters.io.as_ref().and_then(|io| io.read_bytes);
    payload.io_write_bytes = counters.io.as_ref().and_then(|io| io.write_bytes);
    payload.pids_current = counters.pids_current;
    payload.pids_peak = counters.pids_peak;
}
