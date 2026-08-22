#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SandboxAgentSnapshot {
    pub sb_id: u32,
    pub collected_io_observations: u64,
    pub collected_resource_observations: u64,
    pub source_failures: u64,
    pub dropped_observations: u64,
    pub sent_batches: u64,
    pub reconnects: u64,
}
