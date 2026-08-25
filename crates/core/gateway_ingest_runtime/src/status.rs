#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct GatewayIngestStatus {
    pub shutdown_requested: bool,
    pub active_connections: u32,
    pub accepted_connections: u64,
    pub rejected_connections: u64,
    pub closed_connections: u64,
    pub heartbeats: u64,
    pub delivered_batches: u64,
    pub delivered_observations: u64,
    pub delivery_failures: u64,
}
