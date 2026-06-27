use model_core::ids::TraceId;
use model_core::payload::PayloadSegment;
use model_core::trace::TraceRecord;
use semantic_action::{SemanticAction, SemanticActionLink};

use plugin_system::{ObservationConsumer, PluginInstanceStatus};

use crate::ExportError;

use super::subscription::SemanticActionSubscriptionManager;

const EXPORT_RUNTIME_ERROR: &str = "export_runtime";

pub struct SemanticActionExportBatch<'a> {
    pub trace: &'a TraceRecord,
    pub actions: &'a [SemanticAction],
    pub links: &'a [SemanticActionLink],
    pub payload_segments: &'a [PayloadSegment],
}

pub struct ExportRuntime {
    subscriptions: SemanticActionSubscriptionManager,
}

impl ExportRuntime {
    pub fn new(consumers: Vec<Box<dyn ObservationConsumer>>) -> Self {
        Self {
            subscriptions: SemanticActionSubscriptionManager::new(consumers),
        }
    }

    pub fn publish_semantic_actions(
        &self,
        batch: SemanticActionExportBatch<'_>,
    ) -> Result<ExportPublishReport, ExportError> {
        validate_batch(&batch)?;
        Ok(self.subscriptions.publish_semantic_actions(batch))
    }

    pub fn consumer_instance_ids(&self) -> Vec<String> {
        self.subscriptions.consumer_instance_ids()
    }

    pub fn plugin_statuses(&self) -> Vec<PluginInstanceStatus> {
        self.subscriptions.plugin_statuses()
    }

    pub fn payload_snapshot_limit(&self) -> Option<usize> {
        self.subscriptions.payload_snapshot_limit()
    }

    pub fn add_observation_consumer(
        &mut self,
        consumer: Box<dyn ObservationConsumer>,
        warnings: Vec<String>,
    ) -> Result<PluginInstanceStatus, ExportError> {
        self.subscriptions
            .add_observation_consumer(consumer, warnings)
    }

    pub fn remove_observation_consumer(
        &mut self,
        instance_id: &str,
    ) -> Result<ObservationConsumerRemoval, ExportError> {
        self.subscriptions.remove_observation_consumer(instance_id)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExportPublishReport {
    pub dropped_records: Vec<ExportDroppedRecord>,
}

impl ExportPublishReport {
    pub(crate) fn from_dropped_records(dropped_records: Vec<ExportDroppedRecord>) -> Self {
        Self { dropped_records }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservationConsumerRemoval {
    pub status: PluginInstanceStatus,
    pub drop_report: ExportPublishReport,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExportDroppedRecord {
    pub trace_id: TraceId,
    pub exporter: String,
    pub reason: String,
    pub queue_capacity: Option<u32>,
    pub dropped_records: u64,
}

fn validate_batch(batch: &SemanticActionExportBatch<'_>) -> Result<(), ExportError> {
    for action in batch.actions {
        if action.trace_id != batch.trace.trace_id {
            return Err(ExportError::new(
                EXPORT_RUNTIME_ERROR,
                "semantic action trace_id does not match export trace",
            ));
        }
    }
    for link in batch.links {
        if link.trace_id != batch.trace.trace_id {
            return Err(ExportError::new(
                EXPORT_RUNTIME_ERROR,
                "semantic action link trace_id does not match export trace",
            ));
        }
    }
    Ok(())
}
