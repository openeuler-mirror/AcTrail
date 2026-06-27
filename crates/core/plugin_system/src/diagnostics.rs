use model_core::ids::TraceId;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PluginDroppedRecord {
    pub trace_id: TraceId,
    pub plugin_instance: String,
    pub reason: String,
    pub queue_capacity: Option<u32>,
    pub dropped_records: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PluginRuntimeError {
    pub code: String,
    pub message: String,
}

impl PluginRuntimeError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}
