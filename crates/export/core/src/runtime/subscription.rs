use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use plugin_system::{
    ObservationConsumer, PluginInstanceStatus, PluginLifecycleState, PostTraceTask,
};

use crate::ExportError;

use super::report_accumulator::ReportAccumulator;
use super::subscription_slot::ObservationConsumerSlot;
use super::{
    ExportPublishReport, ObservationConsumerRemoval, PostTraceCompletion, SemanticActionExportBatch,
};

pub(crate) struct SemanticActionSubscriptionManager {
    consumers: Vec<ObservationConsumerSlot>,
    semantic_consumer_count: usize,
    post_trace_completion_sender: Sender<PostTraceCompletion>,
    post_trace_completion_receiver: Receiver<PostTraceCompletion>,
}

impl SemanticActionSubscriptionManager {
    pub(crate) fn new(consumers: Vec<Box<dyn ObservationConsumer>>) -> Self {
        let (post_trace_completion_sender, post_trace_completion_receiver) = channel();
        let consumers = consumers
            .into_iter()
            .map(|consumer| {
                ObservationConsumerSlot::new(
                    consumer,
                    Vec::new(),
                    post_trace_completion_sender.clone(),
                )
            })
            .collect::<Vec<_>>();
        let semantic_consumer_count = consumers
            .iter()
            .filter(|slot| slot.receives_semantic_action_batch())
            .count();
        Self {
            consumers,
            semantic_consumer_count,
            post_trace_completion_sender,
            post_trace_completion_receiver,
        }
    }

    pub(crate) fn has_semantic_consumers(&self) -> bool {
        self.semantic_consumer_count != 0
    }

    pub(crate) fn consumer_instance_ids(&self) -> Vec<String> {
        self.consumers
            .iter()
            .map(|slot| slot.instance_id().to_string())
            .collect()
    }

    pub(crate) fn post_trace_instance_ids(&self) -> Vec<String> {
        self.consumers
            .iter()
            .filter(|slot| slot.has_post_trace_analyzer())
            .map(|slot| slot.instance_id().to_string())
            .collect()
    }

    pub(crate) fn enqueue_post_trace(
        &self,
        instance_id: &str,
        task: PostTraceTask,
    ) -> Result<(), ExportError> {
        let slot = self
            .consumers
            .iter()
            .find(|slot| slot.instance_id() == instance_id)
            .ok_or_else(|| {
                ExportError::new(
                    "post_trace_plugin_missing",
                    format!("post-trace plugin instance {instance_id} not found"),
                )
            })?;
        slot.enqueue_post_trace(task)
    }

    pub(crate) fn cancel_post_trace(&self, instance_id: &str) -> Result<(), ExportError> {
        let slot = self
            .consumers
            .iter()
            .find(|slot| slot.instance_id() == instance_id)
            .ok_or_else(|| {
                ExportError::new(
                    "post_trace_plugin_missing",
                    format!("post-trace plugin instance {instance_id} not found"),
                )
            })?;
        slot.cancel_post_trace()
    }

    pub(crate) fn drain_post_trace_completions(&self) -> Vec<PostTraceCompletion> {
        self.post_trace_completion_receiver.try_iter().collect()
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
        let slot = ObservationConsumerSlot::new(
            consumer,
            warnings,
            self.post_trace_completion_sender.clone(),
        );
        let status = slot.status(PluginLifecycleState::Active);
        if slot.receives_semantic_action_batch() {
            self.semantic_consumer_count += 1;
        }
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
        if slot.receives_semantic_action_batch() {
            debug_assert!(self.semantic_consumer_count != 0);
            self.semantic_consumer_count -= 1;
        }
        let runtime_failures = slot.stop();
        let status = slot.status(PluginLifecycleState::Stopped);
        let mut report = ReportAccumulator::default();
        for failure in runtime_failures {
            report.record_runtime_failure(failure);
        }
        slot.drain_pending_reports(&mut report);
        Ok(ObservationConsumerRemoval {
            status,
            drop_report: report.into_report(),
        })
    }

    pub(crate) fn shutdown_observation_consumers(
        &mut self,
        timeout: Duration,
    ) -> ExportPublishReport {
        let started_at = Instant::now();
        let deadline = started_at.checked_add(timeout).unwrap_or(started_at);
        let mut report = ReportAccumulator::default();
        for slot in &mut self.consumers {
            for failure in slot.stop_before(Some(deadline)) {
                report.record_runtime_failure(failure);
            }
        }
        self.drain_pending_reports(&mut report);
        report.into_report()
    }

    pub(crate) fn publish_semantic_actions(
        &self,
        batch: SemanticActionExportBatch<'_>,
    ) -> ExportPublishReport {
        let mut report = ReportAccumulator::default();
        self.drain_pending_reports(&mut report);
        for slot in &self.consumers {
            if !slot.receives_semantic_action_batch() {
                continue;
            }
            slot.publish(&batch, &mut report);
        }
        self.drain_pending_reports(&mut report);
        report.into_report()
    }

    fn drain_pending_reports(&self, report: &mut ReportAccumulator) {
        for slot in &self.consumers {
            slot.drain_pending_reports(report);
        }
    }
}
