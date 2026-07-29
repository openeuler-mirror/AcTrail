use std::time::SystemTime;

use export_core::ExportPublishReport;
use model_core::diagnostics::{DiagnosticKind, DiagnosticRecord, DiagnosticSeverity};
use model_core::ids::DiagnosticId;

const SEMANTIC_EXPORT_DROP_MESSAGE: &str = "semantic action exporter dropped records";
const SEMANTIC_EXPORT_RUNTIME_FAILURE_MESSAGE: &str = "semantic action exporter runtime failure";
const EXPORTER_KEY: &str = "exporter";
const REASON_KEY: &str = "reason";
const DROPPED_RECORDS_KEY: &str = "dropped_records";
const FAILURE_COUNT_KEY: &str = "failure_count";
const QUEUE_CAPACITY_KEY: &str = "queue_capacity";

pub(crate) fn export_drop_diagnostics<E>(
    report: ExportPublishReport,
    emitted_at: SystemTime,
    mut next_diagnostic_id: impl FnMut() -> Result<DiagnosticId, E>,
) -> Result<Vec<DiagnosticRecord>, E> {
    let mut diagnostics = Vec::with_capacity(
        report
            .dropped_records
            .len()
            .saturating_add(report.runtime_failures.len()),
    );
    for dropped in report.dropped_records {
        let mut diagnostic = DiagnosticRecord::new(
            next_diagnostic_id()?,
            dropped.trace_id,
            DiagnosticKind::RuntimeDropped,
            DiagnosticSeverity::Warning,
            emitted_at,
            SEMANTIC_EXPORT_DROP_MESSAGE,
        )
        .with_metadata(EXPORTER_KEY, dropped.exporter)
        .with_metadata(REASON_KEY, dropped.reason)
        .with_metadata(DROPPED_RECORDS_KEY, dropped.dropped_records.to_string());
        if let Some(queue_capacity) = dropped.queue_capacity {
            diagnostic = diagnostic.with_metadata(QUEUE_CAPACITY_KEY, queue_capacity.to_string());
        }
        diagnostics.push(diagnostic);
    }
    for failure in report.runtime_failures {
        let mut diagnostic = DiagnosticRecord::new(
            next_diagnostic_id()?,
            failure.trace_id,
            DiagnosticKind::RuntimeFailure,
            DiagnosticSeverity::Error,
            emitted_at,
            SEMANTIC_EXPORT_RUNTIME_FAILURE_MESSAGE,
        )
        .with_metadata(EXPORTER_KEY, failure.exporter)
        .with_metadata(REASON_KEY, failure.reason)
        .with_metadata(FAILURE_COUNT_KEY, failure.occurrences.to_string());
        if let Some(queue_capacity) = failure.queue_capacity {
            diagnostic = diagnostic.with_metadata(QUEUE_CAPACITY_KEY, queue_capacity.to_string());
        }
        diagnostics.push(diagnostic);
    }
    Ok(diagnostics)
}
