use model_core::{host_identity::HostBootId, ids::TraceId};
use std::time::SystemTime;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResourceKind {
    Host,
    Workload,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceQuery {
    pub kind: ResourceKind,
    pub host_boot_id: Option<HostBootId>,
    pub trace_id: Option<TraceId>,
    pub since: Option<SystemTime>,
    pub until: Option<SystemTime>,
    pub limit: u32,
}
impl ResourceQuery {
    pub fn new(kind: ResourceKind, limit: u32) -> Self {
        Self {
            kind,
            host_boot_id: None,
            trace_id: None,
            since: None,
            until: None,
            limit,
        }
    }
}
