//! Diagnostic-read contracts.

use model_core::diagnostics::DiagnosticRecord;
use model_core::ids::TraceId;

use crate::ReadError;

pub trait DiagnosticReadStore {
    fn list_diagnostics(&self, trace_id: TraceId) -> Result<Vec<DiagnosticRecord>, ReadError>;
}
