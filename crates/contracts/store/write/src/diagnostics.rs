//! Diagnostic-write contracts.

use model_core::diagnostics::DiagnosticRecord;

use crate::WriteError;

pub trait DiagnosticWriteStore {
    fn append_diagnostic(&mut self, diagnostic: DiagnosticRecord) -> Result<(), WriteError>;
}
