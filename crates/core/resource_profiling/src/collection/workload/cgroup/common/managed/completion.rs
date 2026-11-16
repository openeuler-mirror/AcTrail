use crate::ResourceSampleDraft;
use model_core::ids::TraceId;

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
