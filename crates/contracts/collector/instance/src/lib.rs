//! Lifecycle contracts for live collector instances.

use collector_binding::{
    CoverageGuardHandle, CoverageGuardRequest, TraceBindingHandle, TraceBindingRequest,
};
use collector_capability::CollectorDescriptor;
use collector_event::RawCollectorEvent;
use collector_stats::CollectorStats;
use model_core::ids::TraceId;
use payload_event::RawPayloadSegment;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CollectorError {
    pub stage: String,
    pub message: String,
}

impl CollectorError {
    pub fn new(stage: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            stage: stage.into(),
            message: message.into(),
        }
    }
}

pub trait CollectorInstance {
    fn descriptor(&self) -> &CollectorDescriptor;
    fn install_coverage_guard(
        &mut self,
        request: &CoverageGuardRequest,
    ) -> Result<CoverageGuardHandle, CollectorError>;
    fn bind_trace(
        &mut self,
        request: &TraceBindingRequest,
    ) -> Result<TraceBindingHandle, CollectorError>;
    fn unbind_trace(&mut self, trace_id: TraceId) -> Result<(), CollectorError>;
    fn poll_events(&mut self) -> Result<Vec<RawCollectorEvent>, CollectorError> {
        Ok(self.poll_batch()?.observations)
    }
    fn poll_batch(&mut self) -> Result<CollectorPollBatch, CollectorError>;
    fn stats(&self) -> CollectorStats;
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CollectorPollBatch {
    pub observations: Vec<RawCollectorEvent>,
    pub payload_segments: Vec<RawPayloadSegment>,
}
