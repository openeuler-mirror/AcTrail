use std::sync::Arc;

use sandbox_observation::Observation;

use crate::{SandboxEvidenceBatchError, SandboxEvidenceSourceError};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SandboxEvidenceSource {
    gateway_id: u32,
    sb_id: u32,
}

impl SandboxEvidenceSource {
    pub const fn new(gateway_id: u32, sb_id: u32) -> Result<Self, SandboxEvidenceSourceError> {
        if gateway_id == 0 {
            return Err(SandboxEvidenceSourceError::ZeroGatewayId);
        }
        if sb_id == 0 {
            return Err(SandboxEvidenceSourceError::ZeroSbId);
        }
        Ok(Self { gateway_id, sb_id })
    }

    pub const fn gateway_id(self) -> u32 {
        self.gateway_id
    }

    pub const fn sb_id(self) -> u32 {
        self.sb_id
    }
}

/// An owned immutable view containing only observations proven to have no plugin interest.
#[derive(Clone, Debug)]
pub struct NoInterestEvidenceBatch {
    source: SandboxEvidenceSource,
    sequence: u64,
    route_generation: u64,
    observations: Arc<[Observation]>,
    observation_indices: Arc<[u32]>,
    observation_count: u32,
    backing_observation_count: u32,
}

impl NoInterestEvidenceBatch {
    pub fn new(
        source: SandboxEvidenceSource,
        sequence: u64,
        route_generation: u64,
        observations: Arc<[Observation]>,
        observation_indices: Arc<[u32]>,
    ) -> Result<Self, SandboxEvidenceBatchError> {
        if observation_indices.is_empty() {
            return Err(SandboxEvidenceBatchError::Empty);
        }
        let observation_count = u32::try_from(observation_indices.len())
            .map_err(|_| SandboxEvidenceBatchError::ObservationCountOverflow)?;
        let backing_observation_count = u32::try_from(observations.len())
            .map_err(|_| SandboxEvidenceBatchError::ObservationCountOverflow)?;
        let mut previous = None;
        for index in observation_indices.iter().copied() {
            let offset = usize::try_from(index)
                .map_err(|_| SandboxEvidenceBatchError::InvalidObservationIndex(index))?;
            if observations.get(offset).is_none() {
                return Err(SandboxEvidenceBatchError::InvalidObservationIndex(index));
            }
            if previous.is_some_and(|previous| index <= previous) {
                return Err(SandboxEvidenceBatchError::IndicesNotStrictlyIncreasing);
            }
            previous = Some(index);
        }
        Ok(Self {
            source,
            sequence,
            route_generation,
            observations,
            observation_indices,
            observation_count,
            backing_observation_count,
        })
    }

    pub const fn source(&self) -> SandboxEvidenceSource {
        self.source
    }

    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    pub const fn route_generation(&self) -> u64 {
        self.route_generation
    }

    pub fn observation_indices(&self) -> &[u32] {
        &self.observation_indices
    }

    pub fn observation(&self, index: u32) -> Option<&Observation> {
        usize::try_from(index)
            .ok()
            .and_then(|index| self.observations.get(index))
    }

    pub fn observation_count(&self) -> u32 {
        self.observation_count
    }

    pub fn backing_observation_count(&self) -> u32 {
        self.backing_observation_count
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredSandboxEvidence {
    pub record_id: u64,
    pub ingest_epoch: u64,
    pub source: SandboxEvidenceSource,
    pub batch_sequence: u64,
    pub route_generation: u64,
    pub observation_index: u32,
    pub persisted_at_ms: u64,
    pub observation: Observation,
}
