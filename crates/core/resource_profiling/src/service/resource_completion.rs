use model_core::ids::TraceId;
pub struct ResourceCompletion {
    pub trace_id: TraceId,
    pub timed_out: bool,
    pub recovered: bool,
}
#[derive(Default)]
pub struct ResourceCompletionPoll {
    pub waiting: Vec<TraceId>,
    pub ready: Vec<ResourceCompletion>,
}
