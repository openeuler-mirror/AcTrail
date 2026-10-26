use crate::{ProfilingError, VerifiedProcessTarget};
use model_core::{
    ids::TraceId,
    process::{HostProcessCoordinates, ProcessIdentity},
};
#[derive(Clone, Copy)]
pub struct WorkloadTarget {
    pub trace_id: TraceId,
    pub root: ProcessIdentity,
    pub is_container: bool,
    pub requests_resources: bool,
    pub draining: bool,
    pub terminal: bool,
}
pub trait WorkloadTargets {
    fn traces(&self) -> Vec<WorkloadTarget>;
    fn contains_trace(&self, id: TraceId) -> bool;
    fn process_targets(
        &self,
        target: &WorkloadTarget,
        include_children: bool,
    ) -> Vec<VerifiedProcessTarget>;
    fn coordinates(
        &self,
        root: ProcessIdentity,
    ) -> Result<Option<HostProcessCoordinates>, ProfilingError>;
    fn root_is_gone(&self, root: ProcessIdentity) -> Result<bool, ProfilingError>;
}
