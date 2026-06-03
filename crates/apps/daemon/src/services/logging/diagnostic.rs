//! Shared diagnostic logging helpers for daemon services.

use config_core::daemon::DiagnosticLogLevel;

pub(crate) fn diagnostic_log_enabled(
    configured: DiagnosticLogLevel,
    required: DiagnosticLogLevel,
) -> bool {
    configured.enables(required)
}

pub(crate) fn log_diagnostic(
    configured: DiagnosticLogLevel,
    required: DiagnosticLogLevel,
    args: std::fmt::Arguments<'_>,
) {
    if !diagnostic_log_enabled(configured, required) {
        return;
    }
    match required {
        DiagnosticLogLevel::Off => {}
        DiagnosticLogLevel::Info => tracing::info!("{args}"),
        DiagnosticLogLevel::Debug => tracing::debug!("{args}"),
    }
}
