use super::{WorkloadResourceSample, host_resource::HostResourceSample};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "sample", rename_all = "snake_case")]
pub enum ResourceSample {
    Host(HostResourceSample),
    Workload(WorkloadResourceSample),
}
impl ResourceSample {
    pub fn observed_at(&self) -> std::time::SystemTime {
        match self {
            Self::Host(s) => s.observed_at,
            Self::Workload(s) => s.observed_at,
        }
    }
}
