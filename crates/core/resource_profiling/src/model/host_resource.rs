//! Host samples have a boot identity and do not belong to a trace.

use std::time::SystemTime;

use model_core::host_identity::HostBootId;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct HostResourceSample {
    #[serde(with = "super::serde_identity::BootIdentity")]
    pub host_boot_id: HostBootId,
    pub observed_at: SystemTime,
    pub cpu_percent_millis: Option<u64>,
    pub memory_total_bytes: u64,
    pub memory_available_bytes: u64,
    pub memory_free_bytes: u64,
    pub load_average: [f64; 3],
}
