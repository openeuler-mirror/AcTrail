#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SandboxAlertHealth {
    Ready,
    Degraded,
    Stopping,
    Stopped,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SandboxAlertStatus {
    pub schema_version: u32,
    pub ingest_epoch: u64,
    pub health: SandboxAlertHealth,
    pub queue_depth: u64,
    pub queue_capacity: u32,
    pub accepted_alerts: u64,
    pub rejected_alerts: u64,
    pub committed_alerts: u64,
    pub duplicate_alerts: u64,
    pub failed_alerts: u64,
    pub retained_alerts: u64,
    pub last_error: Option<String>,
}

pub trait SandboxAlertStatusPort: Send + Sync + 'static {
    fn status(&self) -> SandboxAlertStatus;
}
