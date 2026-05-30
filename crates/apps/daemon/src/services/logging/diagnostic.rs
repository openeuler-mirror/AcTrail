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
    if diagnostic_log_enabled(configured, required) {
        eprintln!("diagnostic level={} {args}", level_label(required));
    }
}

fn level_label(level: DiagnosticLogLevel) -> &'static str {
    match level {
        DiagnosticLogLevel::Off => "off",
        DiagnosticLogLevel::Info => "info",
        DiagnosticLogLevel::Debug => "debug",
    }
}
