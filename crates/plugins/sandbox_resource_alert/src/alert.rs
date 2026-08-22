use sandbox_observation::{GuestBootId, ProcessMarker};
use sandbox_plugin_delivery::SandboxSource;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SandboxAlert {
    pub source: SandboxSource,
    pub batch_sequence: u64,
    pub kind: SandboxAlertKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SandboxAlertKind {
    OomKilled {
        guest_boot_id: GuestBootId,
        sampled_at_ms: u64,
        previous_count: u64,
        current_count: u64,
        delta: u64,
    },
    OomRisk {
        guest_boot_id: GuestBootId,
        sampled_at_ms: u64,
        available_bytes: u64,
        threshold_bytes: u64,
    },
    HighRead {
        guest_boot_id: GuestBootId,
        process: ProcessMarker,
        sample_started_ms: u64,
        sample_ended_ms: u64,
        bytes: u64,
        threshold_bytes: u64,
    },
    HighWrite {
        guest_boot_id: GuestBootId,
        process: ProcessMarker,
        sample_started_ms: u64,
        sample_ended_ms: u64,
        bytes: u64,
        threshold_bytes: u64,
    },
}
