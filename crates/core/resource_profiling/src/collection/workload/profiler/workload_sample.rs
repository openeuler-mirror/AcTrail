use crate::model::ResourceMetrics;
use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;
use std::time::SystemTime;
#[derive(Clone, Debug)]
pub struct ResourceSampleDraft {
    pub trace_id: TraceId,
    pub observed_at: SystemTime,
    pub process: ProcessIdentity,
    pub payload: ResourceMetrics,
    pub recovered: bool,
}

pub struct ResourceSamplingFailure {
    pub trace_id: TraceId,
    pub message: String,
    pub recovered: bool,
}

#[derive(Default)]
pub struct ResourceMetricsDrain {
    pub samples: Vec<ResourceSampleDraft>,
    pub failures: Vec<ResourceSamplingFailure>,
}
