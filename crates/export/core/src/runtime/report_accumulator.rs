use std::collections::BTreeMap;

use model_core::ids::TraceId;

use super::{ExportDroppedRecord, ExportPublishReport, ExportRuntimeFailure};

const MAX_PENDING_DROP_KEYS: usize = 256;
const MAX_PENDING_REGULAR_DROP_KEYS: usize = MAX_PENDING_DROP_KEYS - 1;
const PENDING_DROP_OVERFLOW_REASON: &str = "pending_drop_accumulator_overflow";
const MAX_PENDING_RUNTIME_FAILURE_KEYS: usize = 256;
const MAX_PENDING_REGULAR_RUNTIME_FAILURE_KEYS: usize = MAX_PENDING_RUNTIME_FAILURE_KEYS - 1;
const PENDING_RUNTIME_FAILURE_OVERFLOW_REASON: &str =
    "pending_runtime_failure_accumulator_overflow";

#[derive(Default)]
pub(super) struct ReportAccumulator {
    dropped: BTreeMap<RouteReportKey, u64>,
    runtime_failures: BTreeMap<RouteReportKey, u64>,
}

impl ReportAccumulator {
    pub(super) fn record_drop(
        &mut self,
        trace_id: Option<TraceId>,
        route: String,
        reason: String,
        queue_capacity: Option<u32>,
        dropped_records: u64,
    ) {
        let key = RouteReportKey {
            trace_id,
            route,
            reason,
            queue_capacity,
        };
        self.dropped
            .entry(key)
            .and_modify(|count| *count = count.saturating_add(dropped_records))
            .or_insert(dropped_records);
    }

    pub(super) fn record_runtime_failure(&mut self, failure: ExportRuntimeFailure) {
        if failure.occurrences == u64::default() {
            return;
        }
        let key = RouteReportKey {
            trace_id: failure.trace_id,
            route: failure.exporter,
            reason: failure.reason,
            queue_capacity: failure.queue_capacity,
        };
        self.runtime_failures
            .entry(key)
            .and_modify(|count| *count = count.saturating_add(failure.occurrences))
            .or_insert(failure.occurrences);
    }

    pub(super) fn into_report(self) -> ExportPublishReport {
        let dropped_records = self
            .dropped
            .into_iter()
            .map(|(key, dropped_records)| ExportDroppedRecord {
                trace_id: key.trace_id,
                exporter: key.route,
                reason: key.reason,
                queue_capacity: key.queue_capacity,
                dropped_records,
            })
            .collect();
        let runtime_failures = self
            .runtime_failures
            .into_iter()
            .map(|(key, occurrences)| ExportRuntimeFailure {
                trace_id: key.trace_id,
                exporter: key.route,
                reason: key.reason,
                queue_capacity: key.queue_capacity,
                occurrences,
            })
            .collect();
        ExportPublishReport {
            dropped_records,
            runtime_failures,
        }
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct RouteReportKey {
    trace_id: Option<TraceId>,
    route: String,
    reason: String,
    queue_capacity: Option<u32>,
}

pub(super) struct PendingDropAccumulator {
    dropped: BTreeMap<RouteReportKey, u64>,
    overflow_exporter: String,
    overflow_dropped_records: u64,
}

impl PendingDropAccumulator {
    pub(super) fn new(overflow_exporter: String) -> Self {
        Self {
            dropped: BTreeMap::new(),
            overflow_exporter,
            overflow_dropped_records: 0,
        }
    }

    pub(super) fn record(&mut self, drop: ExportDroppedRecord) {
        if drop.dropped_records == u64::default() {
            return;
        }
        let key = RouteReportKey {
            trace_id: drop.trace_id,
            route: drop.exporter,
            reason: drop.reason,
            queue_capacity: drop.queue_capacity,
        };
        if let Some(count) = self.dropped.get_mut(&key) {
            *count = count.saturating_add(drop.dropped_records);
            return;
        }
        if self.dropped.len() < MAX_PENDING_REGULAR_DROP_KEYS {
            self.dropped.insert(key, drop.dropped_records);
            return;
        }
        self.overflow_dropped_records = self
            .overflow_dropped_records
            .saturating_add(drop.dropped_records);
    }

    pub(super) fn drain_into(&mut self, report: &mut ReportAccumulator) {
        for (key, dropped_records) in std::mem::take(&mut self.dropped) {
            report.record_drop(
                key.trace_id,
                key.route,
                key.reason,
                key.queue_capacity,
                dropped_records,
            );
        }
        let overflow_dropped_records = std::mem::take(&mut self.overflow_dropped_records);
        if overflow_dropped_records > 0 {
            report.record_drop(
                None,
                self.overflow_exporter.clone(),
                PENDING_DROP_OVERFLOW_REASON.to_string(),
                None,
                overflow_dropped_records,
            );
        }
    }
}

pub(super) struct PendingRuntimeFailureAccumulator {
    failures: BTreeMap<RouteReportKey, u64>,
    overflow_exporter: String,
    overflow_occurrences: u64,
}

impl PendingRuntimeFailureAccumulator {
    pub(super) fn new(overflow_exporter: String) -> Self {
        Self {
            failures: BTreeMap::new(),
            overflow_exporter,
            overflow_occurrences: 0,
        }
    }

    pub(super) fn record(&mut self, failure: ExportRuntimeFailure) {
        if failure.occurrences == u64::default() {
            return;
        }
        let key = RouteReportKey {
            trace_id: failure.trace_id,
            route: failure.exporter,
            reason: failure.reason,
            queue_capacity: failure.queue_capacity,
        };
        if let Some(count) = self.failures.get_mut(&key) {
            *count = count.saturating_add(failure.occurrences);
            return;
        }
        if self.failures.len() < MAX_PENDING_REGULAR_RUNTIME_FAILURE_KEYS {
            self.failures.insert(key, failure.occurrences);
            return;
        }
        self.overflow_occurrences = self
            .overflow_occurrences
            .saturating_add(failure.occurrences);
    }

    pub(super) fn drain_into(&mut self, report: &mut ReportAccumulator) {
        for (key, occurrences) in std::mem::take(&mut self.failures) {
            report.record_runtime_failure(ExportRuntimeFailure {
                trace_id: key.trace_id,
                exporter: key.route,
                reason: key.reason,
                queue_capacity: key.queue_capacity,
                occurrences,
            });
        }
        let overflow_occurrences = std::mem::take(&mut self.overflow_occurrences);
        if overflow_occurrences > 0 {
            report.record_runtime_failure(ExportRuntimeFailure {
                trace_id: None,
                exporter: self.overflow_exporter.clone(),
                reason: PENDING_RUNTIME_FAILURE_OVERFLOW_REASON.to_string(),
                queue_capacity: None,
                occurrences: overflow_occurrences,
            });
        }
    }
}
