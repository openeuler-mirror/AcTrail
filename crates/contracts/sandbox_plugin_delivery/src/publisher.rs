use std::sync::Arc;

use sandbox_observation::Observation;

use crate::{
    SandboxConsumeError, SandboxConsumeReport, SandboxPublishError, SandboxPublishReport,
    SandboxRoutePlan, SandboxSource,
};

#[derive(Clone, Debug)]
pub struct SandboxPublishBatch {
    source: SandboxSource,
    sequence: u64,
    observations: Arc<[Observation]>,
}

impl SandboxPublishBatch {
    pub fn new(source: SandboxSource, sequence: u64, observations: Arc<[Observation]>) -> Self {
        Self {
            source,
            sequence,
            observations,
        }
    }

    pub const fn source(&self) -> SandboxSource {
        self.source
    }

    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    pub fn observations(&self) -> &Arc<[Observation]> {
        &self.observations
    }
}

#[derive(Clone, Debug)]
pub struct SandboxConsumerBatch {
    source: SandboxSource,
    sequence: u64,
    observations: Arc<[Observation]>,
    observation_indices: Arc<[u32]>,
}

impl SandboxConsumerBatch {
    pub fn new(
        source: SandboxSource,
        sequence: u64,
        observations: Arc<[Observation]>,
        observation_indices: Arc<[u32]>,
    ) -> Self {
        Self {
            source,
            sequence,
            observations,
            observation_indices,
        }
    }

    pub const fn source(&self) -> SandboxSource {
        self.source
    }

    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    pub fn observation_indices(&self) -> &[u32] {
        &self.observation_indices
    }

    pub fn observation(&self, index: u32) -> Option<&Observation> {
        usize::try_from(index)
            .ok()
            .and_then(|index| self.observations.get(index))
    }

    pub fn observations(&self) -> impl Iterator<Item = &Observation> {
        self.observation_indices
            .iter()
            .filter_map(|index| self.observation(*index))
    }
}

pub trait SandboxObservationConsumer: Send + Sync + 'static {
    fn consume(
        &self,
        batch: SandboxConsumerBatch,
    ) -> Result<SandboxConsumeReport, SandboxConsumeError>;
}

pub trait SandboxPluginPublisher: Send + Sync {
    fn publish(
        &self,
        batch: SandboxPublishBatch,
        plan: SandboxRoutePlan,
    ) -> Result<SandboxPublishReport, SandboxPublishError>;
}
