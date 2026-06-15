//! Diagnostic logging facade for attach-service internals.

use config_core::daemon::DiagnosticLogLevel;

use crate::services::diagnostic_logging;

use super::StorageAttachService;

impl StorageAttachService {
    pub(in crate::services) fn diagnostic_log_enabled(&self, required: DiagnosticLogLevel) -> bool {
        diagnostic_logging::diagnostic_log_enabled(self.diagnostic_log_level, required)
    }

    pub(in crate::services) fn log_diagnostic(
        &self,
        required: DiagnosticLogLevel,
        args: std::fmt::Arguments<'_>,
    ) {
        diagnostic_logging::log_diagnostic(self.diagnostic_log_level, required, args);
    }
}
