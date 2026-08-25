use crate::GuestBootId;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CpuSnapshot {
    pub total_ticks: u64,
    pub idle_ticks: u64,
    pub logical_cpu_count: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemorySnapshot {
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub used_bytes: u64,
    pub oom_kill_count: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GuestResourceSnapshot {
    pub guest_boot_id: GuestBootId,
    pub sampled_at_ms: u64,
    pub cpu: CpuSnapshot,
    pub memory: MemorySnapshot,
}
