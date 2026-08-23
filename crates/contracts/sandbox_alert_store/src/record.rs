use sandbox_observation::{GuestBootId, ProcessMarker};

use crate::SandboxAlertSourceError;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SandboxAlertSource {
    gateway_id: u32,
    sb_id: u32,
}

impl SandboxAlertSource {
    pub const fn new(gateway_id: u32, sb_id: u32) -> Result<Self, SandboxAlertSourceError> {
        if gateway_id == 0 {
            return Err(SandboxAlertSourceError::ZeroGatewayId);
        }
        if sb_id == 0 {
            return Err(SandboxAlertSourceError::ZeroSbId);
        }
        Ok(Self { gateway_id, sb_id })
    }

    pub const fn gateway_id(self) -> u32 {
        self.gateway_id
    }

    pub const fn sb_id(self) -> u32 {
        self.sb_id
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SandboxAlertKind {
    HighCpu {
        guest_boot_id: GuestBootId,
        sampled_at_ms: u64,
        usage_basis_points: u16,
        threshold_basis_points: u16,
    },
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

impl SandboxAlertKind {
    pub const fn detected_at_ms(self) -> u64 {
        match self {
            Self::HighCpu { sampled_at_ms, .. }
            | Self::OomKilled { sampled_at_ms, .. }
            | Self::OomRisk { sampled_at_ms, .. } => sampled_at_ms,
            Self::HighRead {
                sample_ended_ms, ..
            }
            | Self::HighWrite {
                sample_ended_ms, ..
            } => sample_ended_ms,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SandboxAlertRecord {
    source: SandboxAlertSource,
    batch_sequence: u64,
    observation_index: u32,
    kind: SandboxAlertKind,
}

impl SandboxAlertRecord {
    pub const fn new(
        source: SandboxAlertSource,
        batch_sequence: u64,
        observation_index: u32,
        kind: SandboxAlertKind,
    ) -> Self {
        Self {
            source,
            batch_sequence,
            observation_index,
            kind,
        }
    }

    pub const fn source(self) -> SandboxAlertSource {
        self.source
    }

    pub const fn batch_sequence(self) -> u64 {
        self.batch_sequence
    }

    pub const fn observation_index(self) -> u32 {
        self.observation_index
    }

    pub const fn kind(self) -> SandboxAlertKind {
        self.kind
    }

    pub const fn detected_at_ms(self) -> u64 {
        self.kind.detected_at_ms()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StoredSandboxAlert {
    pub alert_id: u64,
    pub ingest_epoch: u64,
    pub persisted_at_ms: u64,
    pub alert: SandboxAlertRecord,
}
