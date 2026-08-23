use std::sync::Arc;

use gateway_ingest_runtime::{SandboxObservationSink, SinkDeliveryError};
use sandbox_evidence_store::{
    NoInterestEvidenceBatch, SandboxEvidenceAdmission, SandboxEvidenceSource,
    SandboxEvidenceWritePort,
};
use sandbox_observation::{Observation, ObservationBatch};
use sandbox_plugin_delivery::{
    SandboxDeliveryOutcome, SandboxIntentQueryResult, SandboxObservationDescriptors,
    SandboxPluginIntentMatcher, SandboxPluginPublisher, SandboxPublishBatch,
    SandboxRegistryGeneration, SandboxSource,
};

pub(crate) struct SandboxPluginRouteSink {
    matcher: Arc<dyn SandboxPluginIntentMatcher>,
    publisher: Arc<dyn SandboxPluginPublisher>,
    archive: Arc<dyn SandboxEvidenceWritePort>,
}

impl SandboxPluginRouteSink {
    pub(crate) fn new(
        matcher: Arc<dyn SandboxPluginIntentMatcher>,
        publisher: Arc<dyn SandboxPluginPublisher>,
        archive: Arc<dyn SandboxEvidenceWritePort>,
    ) -> Self {
        Self {
            matcher,
            publisher,
            archive,
        }
    }

    fn archive_unmatched(
        &self,
        source: SandboxEvidenceSource,
        sequence: u64,
        generation: SandboxRegistryGeneration,
        observations: Arc<[Observation]>,
        indices: Arc<[u32]>,
    ) -> Result<(), SinkDeliveryError> {
        let batch =
            NoInterestEvidenceBatch::new(source, sequence, generation.get(), observations, indices)
                .map_err(|error| SinkDeliveryError::new("archive_batch", format!("{error:?}")))?;
        match self.archive.try_append_batch(batch) {
            SandboxEvidenceAdmission::Accepted { .. } => Ok(()),
            outcome => Err(SinkDeliveryError::new(
                "archive_admission",
                format!("sandbox evidence writer rejected batch: {outcome:?}"),
            )),
        }
    }

    fn finish_branches(
        plugin: Option<SinkDeliveryError>,
        archive: Result<(), SinkDeliveryError>,
    ) -> Result<(), SinkDeliveryError> {
        match (plugin, archive.err()) {
            (None, None) => Ok(()),
            (Some(error), None) | (None, Some(error)) => Err(error),
            (Some(plugin), Some(archive)) => Err(SinkDeliveryError::new(
                "route_delivery",
                format!("plugin branch failed: {plugin}; archive branch failed: {archive}"),
            )),
        }
    }
}

impl SandboxObservationSink for SandboxPluginRouteSink {
    fn deliver(
        &self,
        gateway_id: u32,
        sb_id: u32,
        batch: ObservationBatch,
    ) -> Result<(), SinkDeliveryError> {
        let source = SandboxSource::new(gateway_id, sb_id)
            .map_err(|error| SinkDeliveryError::new("source", format!("{error:?}")))?;
        let evidence_source = SandboxEvidenceSource::new(gateway_id, sb_id)
            .map_err(|error| SinkDeliveryError::new("source", format!("{error:?}")))?;
        let descriptors = SandboxObservationDescriptors::from_batch(&batch)
            .map_err(|error| SinkDeliveryError::new("descriptor", error))?;
        let query = self
            .matcher
            .query_intent(descriptors.as_slice())
            .map_err(|error| SinkDeliveryError::new("plugin_match", format!("{error:?}")))?;
        let sequence = batch.sequence;
        let observations: Arc<[Observation]> = Arc::from(batch.observations);
        let plan = match query {
            SandboxIntentQueryResult::NoInterest {
                generation,
                observation_count,
            } => {
                let descriptor_count = u32::try_from(descriptors.len()).map_err(|_| {
                    SinkDeliveryError::new("plugin_match", "sandbox descriptor count exceeds u32")
                })?;
                if observation_count != descriptor_count {
                    return Err(SinkDeliveryError::new(
                        "plugin_match",
                        format!(
                            "NoInterest observation count mismatch: matcher={observation_count} batch={descriptor_count}"
                        ),
                    ));
                }
                let indices = Arc::from((0..observation_count).collect::<Vec<_>>());
                return self.archive_unmatched(
                    evidence_source,
                    sequence,
                    generation,
                    observations,
                    indices,
                );
            }
            SandboxIntentQueryResult::Matched(plan) => plan,
        };
        let unmatched =
            (!plan.unmatched_indices().is_empty()).then(|| Arc::from(plan.unmatched_indices()));
        let generation = plan.generation();
        let publish = SandboxPublishBatch::new(source, sequence, Arc::clone(&observations));
        let report = self
            .publisher
            .publish(publish, plan)
            .map_err(|error| SinkDeliveryError::new("plugin_publish", format!("{error:?}")))?;
        let plugin_failure = report
            .deliveries
            .iter()
            .any(|delivery| !matches!(delivery.outcome, SandboxDeliveryOutcome::Accepted { .. }))
            .then(|| {
                SinkDeliveryError::new(
                    "plugin_backpressure",
                    "one or more sandbox plugin queues rejected the batch",
                )
            });
        let archive = unmatched.map_or(Ok(()), |indices| {
            self.archive_unmatched(evidence_source, sequence, generation, observations, indices)
        });
        Self::finish_branches(plugin_failure, archive)
    }
}
