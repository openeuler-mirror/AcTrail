//! Live OTLP JSON exporter backed by selectable delivery targets.

use export_core::{
    BestEffortSemanticActionRoute, BestEffortSemanticActionRouteConfig, ExportError,
    SemanticActionExportAdapter, SemanticActionExportRecord, SemanticActionKindSelection,
};
use export_core::{
    TerminalActionDelivery, TerminalActionGate, TerminalActionLimits, TerminalActionRejection,
};
use plugin_system::{
    ObservationBatch, ObservationConsumeReport, ObservationConsumer, ObservationEventFamily,
    PluginDroppedRecord, PluginRuntimeError,
};
use plugin_system_contracts::PluginRuntimeKind;
use semantic_action::{SemanticActionKind, SemanticActionStatus};

use crate::OtelJsonlExporterConfig;
use crate::exporter::ExporterSink;

const OTEL_JSONL_EXPORTER_NAME: &str = "otel_live_jsonl";
const OTEL_JSONL_PLUGIN_ID: &str = "otel-jsonl";
const WRITER_THREAD_NAME: &str = "actrail-live-otel-export";

type OtelJsonlSemanticActionRoute = BestEffortSemanticActionRoute<OtelJsonlSemanticActionAdapter>;

pub fn build_otel_jsonl_observation_consumer_instance_with_subscriptions(
    instance_id: impl Into<String>,
    config: OtelJsonlExporterConfig,
    event_families: Vec<ObservationEventFamily>,
) -> Result<OtelJsonlObservationConsumer, ExportError> {
    let route = build_otel_jsonl_semantic_action_route(config.exporter, config.queue_capacity)?;
    Ok(OtelJsonlObservationConsumer::new(
        instance_id,
        config.action_kinds,
        config.terminal_action_limits,
        route,
        event_families,
    ))
}

fn build_otel_jsonl_semantic_action_route(
    exporter: crate::config::ExporterConfig,
    queue_capacity: u32,
) -> Result<OtelJsonlSemanticActionRoute, ExportError> {
    let sink = ExporterSink::open(exporter)?;
    BestEffortSemanticActionRoute::spawn(
        OtelJsonlSemanticActionAdapter,
        BestEffortSemanticActionRouteConfig {
            worker_thread_name: WRITER_THREAD_NAME,
            queue_capacity,
            shutdown_timeout: None,
        },
        sink,
    )
}

pub struct OtelJsonlObservationConsumer {
    instance_id: String,
    event_families: Vec<ObservationEventFamily>,
    action_kinds: SemanticActionKindSelection,
    route: OtelJsonlSemanticActionRoute,
    terminal_actions: TerminalActionGate<SemanticActionStatus>,
}

impl OtelJsonlObservationConsumer {
    fn consume_batch(
        &self,
        batch: &ObservationBatch<'_>,
    ) -> Result<ObservationConsumeReport, PluginRuntimeError> {
        validate_observation_batch(batch)?;
        let mut dropped_records = Vec::new();
        for action in batch.semantic_actions {
            if action.status == SemanticActionStatus::InProgress
                || (action.kind == SemanticActionKind::CommandInvocation
                    && action.completeness == semantic_action::SemanticActionCompleteness::Partial)
                || !self.action_kinds.enabled(action.kind)
            {
                continue;
            }
            let outcome = self.terminal_actions.deliver(
                action.trace_id.get(),
                &action.action_id,
                action.status,
                || {
                    self.route.prepare(SemanticActionExportRecord {
                        trace: batch.trace,
                        action,
                        links: batch.semantic_links,
                    })
                },
                |message| self.route.publish_encoded(message),
            );
            let (reason, capacity, count) = match outcome {
                Ok(TerminalActionDelivery::Skipped(TerminalActionRejection::Duplicate)) => continue,
                Ok(TerminalActionDelivery::Skipped(TerminalActionRejection::Conflict {
                    previous,
                })) => (
                    format!(
                        "otel_jsonl_terminal_conflict: action {} has conflicting terminal revisions: {} then {}",
                        action.action_id,
                        previous.as_str(),
                        action.status.as_str()
                    ),
                    None,
                    1,
                ),
                Ok(TerminalActionDelivery::Skipped(TerminalActionRejection::Capacity(reason))) => {
                    (reason.code().to_string(), None, 1)
                }
                Ok(TerminalActionDelivery::Attempted(result)) => {
                    let Some(drop) = result.dropped_outcome() else {
                        continue;
                    };
                    if drop.dropped_records() == 0 {
                        continue;
                    }
                    (
                        drop.reason().code().to_string(),
                        drop.queue_capacity(),
                        drop.dropped_records(),
                    )
                }
                Err(error) => {
                    let capacity = error.queue_capacity();
                    (format!("{}: {}", error.code, error.message), capacity, 1)
                }
            };

            dropped_records.push(PluginDroppedRecord {
                trace_id: Some(action.trace_id),
                plugin_instance: self.instance_id.clone(),
                reason,
                queue_capacity: capacity,
                dropped_records: count,
            });
        }
        Ok(ObservationConsumeReport {
            dropped_records,
            reevaluate_at: None,
        })
    }

    fn new(
        instance_id: impl Into<String>,
        action_kinds: SemanticActionKindSelection,
        terminal_action_limits: TerminalActionLimits,
        route: OtelJsonlSemanticActionRoute,
        event_families: Vec<ObservationEventFamily>,
    ) -> Self {
        Self {
            instance_id: instance_id.into(),
            event_families,
            action_kinds,
            route,
            terminal_actions: TerminalActionGate::new(terminal_action_limits),
        }
    }
}

impl ObservationConsumer for OtelJsonlObservationConsumer {
    fn instance_id(&self) -> &str {
        &self.instance_id
    }

    fn plugin_id(&self) -> &str {
        OTEL_JSONL_PLUGIN_ID
    }

    fn runtime_kind(&self) -> PluginRuntimeKind {
        PluginRuntimeKind::Builtin
    }

    fn subscribed_event_families(&self) -> Vec<ObservationEventFamily> {
        self.event_families.clone()
    }

    fn consume(
        &self,
        batch: ObservationBatch<'_>,
    ) -> Result<ObservationConsumeReport, PluginRuntimeError> {
        let result = self.consume_batch(&batch);
        // Final cleanup must run even when validation or delivery returns early.
        let cleanup = if batch.trace_finalized {
            self.terminal_actions
                .finish_trace(batch.trace.trace_id.get())
                .map_err(|error| PluginRuntimeError::new(error.code, error.message))
        } else {
            Ok(())
        };
        let report = result?;
        cleanup?;
        Ok(report)
    }

    fn finish(&self) -> Result<ObservationConsumeReport, PluginRuntimeError> {
        let cleanup = self
            .terminal_actions
            .clear()
            .map_err(|error| PluginRuntimeError::new(error.code, error.message));
        let finish = self.route.finish();
        cleanup?;
        let Some(error) = finish.error() else {
            return Ok(ObservationConsumeReport::empty());
        };
        if finish.dropped_records() == u64::default() {
            return Err(PluginRuntimeError::new(
                error.code.clone(),
                error.message.clone(),
            ));
        }
        Ok(ObservationConsumeReport {
            dropped_records: vec![PluginDroppedRecord {
                trace_id: None,
                plugin_instance: self.instance_id.clone(),
                reason: format!("{}: {}", error.code, error.message),
                queue_capacity: error.queue_capacity(),
                dropped_records: finish.dropped_records(),
            }],
            reevaluate_at: None,
        })
    }
}

struct OtelJsonlSemanticActionAdapter;

impl SemanticActionExportAdapter for OtelJsonlSemanticActionAdapter {
    type Message = String;

    fn name(&self) -> &'static str {
        OTEL_JSONL_EXPORTER_NAME
    }

    fn encode(&self, record: SemanticActionExportRecord<'_>) -> Result<Self::Message, ExportError> {
        Ok(export_otel_codec::render_otlp_json_line(
            record.trace,
            record.action,
            record.links,
        ))
    }
}

fn validate_observation_batch(batch: &ObservationBatch<'_>) -> Result<(), PluginRuntimeError> {
    for action in batch.semantic_actions {
        if action.trace_id != batch.trace.trace_id {
            return Err(PluginRuntimeError::new(
                OTEL_JSONL_EXPORTER_NAME,
                "semantic action trace_id does not match observation trace",
            ));
        }
    }
    for link in batch.semantic_links {
        if link.trace_id != batch.trace.trace_id {
            return Err(PluginRuntimeError::new(
                OTEL_JSONL_EXPORTER_NAME,
                "semantic action link trace_id does not match observation trace",
            ));
        }
    }
    Ok(())
}
