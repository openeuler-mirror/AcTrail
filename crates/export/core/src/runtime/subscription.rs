use model_core::payload::PayloadSegment;
use plugin_system::{ObservationConsumer, PluginInstanceStatus, PluginLifecycleState};

use crate::ExportError;

use super::subscription_slot::{DropAccumulator, ObservationConsumerSlot};
use super::{ExportPublishReport, ObservationConsumerRemoval, SemanticActionExportBatch};

pub(crate) struct SemanticActionSubscriptionManager {
    consumers: Vec<ObservationConsumerSlot>,
}

impl SemanticActionSubscriptionManager {
    pub(crate) fn new(consumers: Vec<Box<dyn ObservationConsumer>>) -> Self {
        Self {
            consumers: consumers
                .into_iter()
                .map(|consumer| ObservationConsumerSlot::new(consumer, Vec::new()))
                .collect(),
        }
    }

    pub(crate) fn consumer_instance_ids(&self) -> Vec<String> {
        self.consumers
            .iter()
            .map(|slot| slot.instance_id().to_string())
            .collect()
    }

    pub(crate) fn plugin_statuses(&self) -> Vec<PluginInstanceStatus> {
        self.consumers
            .iter()
            .map(|slot| slot.status(PluginLifecycleState::Active))
            .collect()
    }

    pub(crate) fn add_observation_consumer(
        &mut self,
        consumer: Box<dyn ObservationConsumer>,
        warnings: Vec<String>,
    ) -> Result<PluginInstanceStatus, ExportError> {
        let instance_id = consumer.instance_id().to_string();
        if instance_id.trim().is_empty() {
            return Err(ExportError::new(
                "plugin_runtime",
                "plugin instance id must not be empty",
            ));
        }
        if self
            .consumers
            .iter()
            .any(|existing| existing.instance_id() == instance_id)
        {
            return Err(ExportError::new(
                "plugin_runtime",
                format!("plugin instance {instance_id} already exists"),
            ));
        }
        let slot = ObservationConsumerSlot::new(consumer, warnings);
        let status = slot.status(PluginLifecycleState::Active);
        self.consumers.push(slot);
        Ok(status)
    }

    pub(crate) fn remove_observation_consumer(
        &mut self,
        instance_id: &str,
    ) -> Result<ObservationConsumerRemoval, ExportError> {
        let Some(index) = self
            .consumers
            .iter()
            .position(|slot| slot.instance_id() == instance_id)
        else {
            return Err(ExportError::new(
                "plugin_runtime",
                format!("plugin instance {instance_id} not found"),
            ));
        };
        let mut slot = self.consumers.remove(index);
        slot.stop();
        let status = slot.status(PluginLifecycleState::Stopped);
        let mut dropped = DropAccumulator::default();
        slot.drain_pending_drops(&mut dropped);
        Ok(ObservationConsumerRemoval {
            status,
            drop_report: dropped.into_report(),
        })
    }

    pub(crate) fn publish_semantic_actions(
        &self,
        batch: SemanticActionExportBatch<'_>,
    ) -> ExportPublishReport {
        let mut dropped = DropAccumulator::default();
        self.drain_pending_drops(&mut dropped);
        let mut metadata_payload_segments = None;
        for slot in &self.consumers {
            if !slot.receives_semantic_action_batch() {
                continue;
            }
            let payload_segments = payload_segments_for_consumer(
                slot.payload_snapshot_limit(),
                batch.payload_segments,
                &mut metadata_payload_segments,
            );
            slot.publish(&batch, payload_segments, &mut dropped);
        }
        self.drain_pending_drops(&mut dropped);
        dropped.into_report()
    }

    pub(crate) fn payload_snapshot_limit(&self) -> Option<usize> {
        self.consumers
            .iter()
            .filter(|slot| slot.receives_semantic_action_batch())
            .filter_map(ObservationConsumerSlot::payload_snapshot_limit)
            .max()
    }

    fn drain_pending_drops(&self, dropped: &mut DropAccumulator) {
        for slot in &self.consumers {
            slot.drain_pending_drops(dropped);
        }
    }
}

fn payload_segments_for_consumer<'a>(
    payload_snapshot_limit: Option<usize>,
    payload_segments: &'a [PayloadSegment],
    metadata_payload_segments: &'a mut Option<Vec<PayloadSegment>>,
) -> &'a [PayloadSegment] {
    let Some(limit) = payload_snapshot_limit else {
        return metadata_payload_segments
            .get_or_insert_with(|| payload_metadata_only(payload_segments))
            .as_slice();
    };
    &payload_segments[..limit.min(payload_segments.len())]
}

fn payload_metadata_only(payload_segments: &[PayloadSegment]) -> Vec<PayloadSegment> {
    payload_segments
        .iter()
        .map(|segment| {
            let mut segment = segment.clone();
            segment.bytes.clear();
            segment
        })
        .collect()
}
