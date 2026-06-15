use std::collections::BTreeMap;

use model_core::ids::TraceId;

use crate::{ExportError, SemanticActionExportRecord, SemanticActionExportRoute};

use super::{ExportDroppedRecord, ExportPublishReport, SemanticActionExportBatch};

pub(crate) struct SemanticActionSubscriptionManager {
    routes: Vec<Box<dyn SemanticActionExportRoute>>,
}

impl SemanticActionSubscriptionManager {
    pub(crate) fn new(routes: Vec<Box<dyn SemanticActionExportRoute>>) -> Self {
        Self { routes }
    }

    pub(crate) fn publish_semantic_actions(
        &self,
        batch: SemanticActionExportBatch<'_>,
    ) -> ExportPublishReport {
        let mut dropped = BTreeMap::<RouteDropKey, u64>::new();
        for route in &self.routes {
            let route_name = route.name().to_string();
            for action in batch.actions {
                let record = SemanticActionExportRecord {
                    trace: batch.trace,
                    action,
                    links: batch.links,
                };
                match route.publish(record) {
                    Ok(result) => {
                        let Some(drop) = result.dropped_outcome() else {
                            continue;
                        };
                        if drop.dropped_records() == u64::default() {
                            continue;
                        }
                        record_export_drop(
                            &mut dropped,
                            action.trace_id,
                            route_name.clone(),
                            drop.reason().code().to_string(),
                            drop.queue_capacity(),
                            drop.dropped_records(),
                        );
                    }
                    Err(error) => record_export_error(
                        &mut dropped,
                        action.trace_id,
                        route_name.clone(),
                        error,
                    ),
                }
            }
        }
        ExportPublishReport::from_dropped_records(
            dropped
                .into_iter()
                .map(|(key, dropped_records)| ExportDroppedRecord {
                    trace_id: key.trace_id,
                    exporter: key.route,
                    reason: key.reason,
                    queue_capacity: key.queue_capacity,
                    dropped_records,
                })
                .collect(),
        )
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct RouteDropKey {
    trace_id: TraceId,
    route: String,
    reason: String,
    queue_capacity: Option<u32>,
}

fn record_export_drop(
    dropped: &mut BTreeMap<RouteDropKey, u64>,
    trace_id: TraceId,
    route: String,
    reason: String,
    queue_capacity: Option<u32>,
    dropped_records: u64,
) {
    let key = RouteDropKey {
        trace_id,
        route,
        reason,
        queue_capacity,
    };
    dropped
        .entry(key)
        .and_modify(|count| *count = count.saturating_add(dropped_records))
        .or_insert(dropped_records);
}

fn record_export_error(
    dropped: &mut BTreeMap<RouteDropKey, u64>,
    trace_id: TraceId,
    route: String,
    error: ExportError,
) {
    let queue_capacity = error.queue_capacity();
    record_export_drop(
        dropped,
        trace_id,
        route,
        format!("{}: {}", error.code, error.message),
        queue_capacity,
        1,
    );
}
