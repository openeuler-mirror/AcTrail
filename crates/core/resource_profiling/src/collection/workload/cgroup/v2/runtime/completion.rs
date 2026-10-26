use crate::ResourceSampleDraft;
use crate::model::ResourceMetrics;
use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;
use std::time::SystemTime;
pub struct ResourceFinalizationDraft {
    pub trace_id: TraceId,
    pub sample: ResourceSampleDraft,
    pub timed_out: bool,
    pub recovered: bool,
}

#[derive(Default)]
pub struct ResourceFinalizationPoll {
    pub waiting: Vec<TraceId>,
    pub ready: Vec<ResourceFinalizationDraft>,
}

pub struct ExternalFinalizationDraft {
    pub trace_id: TraceId,
    pub observed_at: SystemTime,
    pub process: ProcessIdentity,
    pub payload: ResourceMetrics,
}

pub(crate) enum ExternalSampleOutcome {
    Emitted(ResourceSampleDraft),
    BecameStale,
    Failed(String),
}
