use crate::ResourceSampleDraft;
use crate::model::ResourceMetrics;
use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;
use std::time::SystemTime;

pub struct CgroupV2BindingFinalization {
    pub trace_id: TraceId,
    pub observed_at: SystemTime,
    pub process: ProcessIdentity,
    pub payload: ResourceMetrics,
}

pub(crate) enum CgroupV2BindingSampleOutcome {
    Emitted(ResourceSampleDraft),
    BecameStale,
    Failed(String),
}
