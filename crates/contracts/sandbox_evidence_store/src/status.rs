#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SandboxEvidenceHealth {
    Ready,
    Degraded,
    Stopping,
    Stopped,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SandboxEvidenceStatus {
    pub schema_version: u32,
    pub ingest_epoch: u64,
    pub health: SandboxEvidenceHealth,
    pub queue_depth: u64,
    pub queue_capacity: u32,
    pub accepted_batches: u64,
    pub accepted_observations: u64,
    pub rejected_batches: u64,
    pub committed_batches: u64,
    pub committed_observations: u64,
    pub duplicate_observations: u64,
    pub failed_batches: u64,
    pub retained_observations: u64,
    pub last_error: Option<String>,
}

pub trait SandboxEvidenceStatusPort: Send + Sync + 'static {
    fn status(&self) -> SandboxEvidenceStatus;
}
