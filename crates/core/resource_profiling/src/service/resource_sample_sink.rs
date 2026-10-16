use crate::{
    ResourceSampleDraft,
    alerts::{ResourceAlertConfig, ResourceAlertEvaluator},
    model::{ResourceSample, WorkloadResourceSample},
    storage::ResourceStorageBackend,
};
pub(super) struct ResourceSampleSink {
    backend: Option<Box<dyn ResourceStorageBackend>>,
    alerts: ResourceAlertEvaluator,
}
impl ResourceSampleSink {
    pub(super) fn new(
        backend: Option<Box<dyn ResourceStorageBackend>>,
        config: ResourceAlertConfig,
    ) -> Self {
        Self {
            backend,
            alerts: ResourceAlertEvaluator::new(config),
        }
    }
    pub(super) fn submit_drafts(&mut self, drafts: Vec<ResourceSampleDraft>) {
        self.submit(
            drafts
                .into_iter()
                .map(|d| {
                    ResourceSample::Workload(WorkloadResourceSample {
                        trace_id: d.trace_id,
                        process: d.process,
                        observed_at: d.observed_at,
                        payload: d.payload,
                    })
                })
                .collect(),
        );
    }
    pub(super) fn submit(&mut self, mut samples: Vec<ResourceSample>) {
        if samples.is_empty() {
            return;
        }
        for sample in &mut samples {
            if let ResourceSample::Workload(sample) = sample {
                self.alerts.evaluate(&mut sample.payload);
            }
        }
        if let Some(backend) = &mut self.backend {
            if let Err(error) = backend.write(&samples, std::time::Duration::MAX) {
                tracing::warn!(%error,"resource sample output failed");
            }
        }
    }
}
