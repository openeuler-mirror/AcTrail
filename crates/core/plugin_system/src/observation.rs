use std::sync::Arc;

use model_core::payload::PayloadSegment;
use model_core::trace::TraceRecord;
use semantic_action::{SemanticAction, SemanticActionLink};
use serde::{Deserialize, Serialize};

use crate::{
    PluginDroppedRecord, PluginHostcallMetricsSource, PluginRuntimeError, PluginRuntimeKind,
};

pub const DEFAULT_OBSERVATION_QUEUE_CAPACITY: u32 = 4096;
pub const DEFAULT_OBSERVATION_EVENT_FAMILIES: [ObservationEventFamily; 2] = [
    ObservationEventFamily::SemanticAction,
    ObservationEventFamily::SemanticActionLink,
];

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ObservationEventFamily {
    SemanticAction,
    SemanticActionLink,
    Diagnostic,
    TraceLifecycle,
    ResourceMetric,
    PayloadMetadata,
}

pub struct ObservationBatch<'a> {
    pub trace: &'a TraceRecord,
    pub semantic_actions: &'a [SemanticAction],
    pub semantic_links: &'a [SemanticActionLink],
    pub payload_segments: &'a [PayloadSegment],
}

pub trait ObservationConsumer: Send + Sync {
    fn instance_id(&self) -> &str;

    fn plugin_id(&self) -> &str;

    fn runtime_kind(&self) -> PluginRuntimeKind;

    fn host_grants(&self) -> Vec<String> {
        Vec::new()
    }

    fn hostcall_metrics_source(&self) -> Option<Arc<dyn PluginHostcallMetricsSource>> {
        None
    }

    fn payload_snapshot_limit(&self) -> Option<usize> {
        None
    }

    fn observation_queue_capacity(&self) -> u32 {
        DEFAULT_OBSERVATION_QUEUE_CAPACITY
    }

    fn subscribed_event_families(&self) -> Vec<ObservationEventFamily> {
        DEFAULT_OBSERVATION_EVENT_FAMILIES.to_vec()
    }

    fn consume(
        &self,
        batch: ObservationBatch<'_>,
    ) -> Result<ObservationConsumeReport, PluginRuntimeError>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservationConsumeReport {
    pub dropped_records: Vec<PluginDroppedRecord>,
}

impl ObservationConsumeReport {
    pub fn empty() -> Self {
        Self {
            dropped_records: Vec::new(),
        }
    }
}
