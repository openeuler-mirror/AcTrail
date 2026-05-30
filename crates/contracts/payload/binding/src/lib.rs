//! Trace-to-payload-adapter binding contracts.

use model_core::ids::{ProfileName, TraceId};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PayloadBindingRequest {
    pub trace_id: TraceId,
    pub profile_name: ProfileName,
    pub allow_sensitive_capture: bool,
}
