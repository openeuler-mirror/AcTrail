//! Realtime OTLP/HTTP observation consumer and delivery boundary.

use std::sync::Arc;

use export_core::{
    BestEffortSemanticActionRoute, BestEffortSemanticActionRouteConfig, ExportError,
    SemanticActionExportRecord, SemanticActionKindSelection,
};
use export_core::{
    TerminalActionDelivery, TerminalActionGate, TerminalActionLimits, TerminalActionRejection,
};
use plugin_system::{
    DEFAULT_OBSERVATION_EVENT_FAMILIES, ObservationBatch, ObservationConsumeReport,
    ObservationConsumer, ObservationEventFamily, PluginDroppedRecord,
    PluginOperationalMetricsSource, PluginRuntimeError,
};
use plugin_system_contracts::PluginRuntimeKind;
use semantic_action::{
    SemanticActionKind, SemanticActionStatus,
    attr_keys::{agent, llm_request, process, process_parent},
};

use crate::batch::OtelHttpSemanticActionAdapter;
use crate::config::{Endpoint, OtelAttributeMode, OtelHttpExporterConfig};
use crate::metrics::OtelHttpOperationalMetrics;
use crate::sink::HttpBatchSink;

pub(super) const OTEL_HTTP_EXPORTER_NAME: &str = "otel_live_http";
const OTEL_HTTP_PLUGIN_ID: &str = "otel-http";
pub const OTEL_HTTP_BUILTIN_PLUGIN_INSTANCE_ID: &str = "builtin.otel-http";
const SENDER_THREAD_NAME: &str = "actrail-live-otel-http";

type OtelHttpSemanticActionRoute = BestEffortSemanticActionRoute<OtelHttpSemanticActionAdapter>;

pub fn build_otel_http_observation_consumer(
    config: OtelHttpExporterConfig,
) -> Result<OtelHttpObservationConsumer, ExportError> {
    build_otel_http_observation_consumer_instance(OTEL_HTTP_BUILTIN_PLUGIN_INSTANCE_ID, config)
}

pub fn build_otel_http_observation_consumer_instance(
    instance_id: impl Into<String>,
    config: OtelHttpExporterConfig,
) -> Result<OtelHttpObservationConsumer, ExportError> {
    build_otel_http_observation_consumer_instance_with_subscriptions(
        instance_id,
        config,
        DEFAULT_OBSERVATION_EVENT_FAMILIES.to_vec(),
    )
}

pub fn build_otel_http_observation_consumer_instance_with_subscriptions(
    instance_id: impl Into<String>,
    config: OtelHttpExporterConfig,
    event_families: Vec<ObservationEventFamily>,
) -> Result<OtelHttpObservationConsumer, ExportError> {
    config
        .validate_enabled_route()
        .map_err(|message| ExportError::new(OTEL_HTTP_EXPORTER_NAME, message))?;
    let endpoint = Endpoint::parse(&config.endpoint)
        .map_err(|message| ExportError::new(OTEL_HTTP_EXPORTER_NAME, message))?;
    let metrics = Arc::new(OtelHttpOperationalMetrics::new(config.queue_capacity));
    let action_kinds = config.action_kinds.clone();
    let attribute_mode = config.attribute_mode;
    let sink = HttpBatchSink::new_with_metrics(endpoint, &config, Arc::clone(&metrics));
    let route = BestEffortSemanticActionRoute::spawn(
        OtelHttpSemanticActionAdapter {
            encoding: config.encoding,
        },
        BestEffortSemanticActionRouteConfig {
            worker_thread_name: SENDER_THREAD_NAME,
            queue_capacity: config.queue_capacity,
            shutdown_timeout: Some(config.shutdown_flush_deadline()),
        },
        sink,
    )?;
    Ok(OtelHttpObservationConsumer::new(
        instance_id,
        action_kinds,
        attribute_mode,
        config.terminal_action_limits,
        route,
        event_families,
        metrics,
    ))
}

pub struct OtelHttpObservationConsumer {
    instance_id: String,
    event_families: Vec<ObservationEventFamily>,
    action_kinds: SemanticActionKindSelection,
    attribute_mode: OtelAttributeMode,
    terminal_actions: TerminalActionGate<SemanticActionStatus>,
    route: OtelHttpSemanticActionRoute,
    metrics: Arc<OtelHttpOperationalMetrics>,
}

impl OtelHttpObservationConsumer {
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
                    let sanitized;
                    let action = match self.attribute_mode {
                        OtelAttributeMode::Full => action,
                        OtelAttributeMode::MetadataOnly => {
                            sanitized = metadata_only_action(action);
                            &sanitized
                        }
                    };

                    self.route.prepare(SemanticActionExportRecord {
                        trace: batch.trace,
                        action,
                        links: batch.semantic_links,
                    })
                },
                |message| {
                    self.metrics.queue_enter();
                    let result = self.route.publish_encoded(message);
                    if result
                        .as_ref()
                        .map_or(true, |result| result.dropped_outcome().is_some())
                    {
                        self.metrics.queue_leave();
                    }
                    result
                },
            );
            let (reason, capacity, count) = match outcome {
                Ok(TerminalActionDelivery::Skipped(TerminalActionRejection::Duplicate)) => continue,
                Ok(TerminalActionDelivery::Skipped(TerminalActionRejection::Conflict {
                    previous,
                })) => (
                    format!(
                        "otel_http_terminal_conflict: action {} has conflicting terminal revisions: {} then {}",
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
            self.metrics.record_inline_drop(count);
            self.metrics.record_error(reason.clone());

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
        attribute_mode: OtelAttributeMode,
        terminal_action_limits: TerminalActionLimits,
        route: OtelHttpSemanticActionRoute,
        event_families: Vec<ObservationEventFamily>,
        metrics: Arc<OtelHttpOperationalMetrics>,
    ) -> Self {
        Self {
            instance_id: instance_id.into(),
            event_families,
            action_kinds,
            attribute_mode,
            terminal_actions: TerminalActionGate::new(terminal_action_limits),
            route,
            metrics,
        }
    }
}

impl ObservationConsumer for OtelHttpObservationConsumer {
    fn instance_id(&self) -> &str {
        &self.instance_id
    }

    fn plugin_id(&self) -> &str {
        OTEL_HTTP_PLUGIN_ID
    }

    fn runtime_kind(&self) -> PluginRuntimeKind {
        PluginRuntimeKind::Builtin
    }

    fn subscribed_event_families(&self) -> Vec<ObservationEventFamily> {
        self.event_families.clone()
    }

    fn operational_metrics_source(&self) -> Option<Arc<dyn PluginOperationalMetricsSource>> {
        Some(self.metrics.clone())
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
        self.metrics.report_finish_drops(finish.dropped_records());
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

fn metadata_only_action(
    action: &semantic_action::SemanticAction,
) -> semantic_action::SemanticAction {
    let mut sanitized = action.clone();
    // Titles are often derived from command lines, paths, tool names, or LLM
    // previews. Use the stable kind as the span name at the safe boundary.
    sanitized.title = action.kind.as_str().to_string();
    let agent_identity = action.kind == SemanticActionKind::AgentIdentity;
    sanitized.attributes.retain(|key, _| {
        matches!(
            key.as_str(),
            process_parent::IDENTITY_STATE
                | llm_request::TRAJECTORY_ID
                | llm_request::TRAJECTORY_INFERENCE_VERSION
        ) || agent_identity
            && matches!(
                key.as_str(),
                agent::IDENTITY_STATUS
                    | agent::IDENTITY_SOURCE
                    | agent::IDENTITY_EVIDENCE_ACTION_ID
                    | agent::TYPE
                    | agent::TYPE_SOURCE
                    | process::EXECUTABLE
            )
    });
    sanitized
}

fn validate_observation_batch(batch: &ObservationBatch<'_>) -> Result<(), PluginRuntimeError> {
    for action in batch.semantic_actions {
        if action.trace_id != batch.trace.trace_id {
            return Err(PluginRuntimeError::new(
                OTEL_HTTP_EXPORTER_NAME,
                "semantic action trace_id does not match observation trace",
            ));
        }
    }
    for link in batch.semantic_links {
        if link.trace_id != batch.trace.trace_id {
            return Err(PluginRuntimeError::new(
                OTEL_HTTP_EXPORTER_NAME,
                "semantic action link trace_id does not match observation trace",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../../../../tests/unit/crates/export/adapters/otel_http/live_http/request_body_export_tests.rs"]
mod request_body_export_tests;
