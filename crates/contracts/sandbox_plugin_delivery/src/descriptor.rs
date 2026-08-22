use sandbox_observation::{Observation, ObservationBatch};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum SandboxObservationKind {
    ProcessIo,
    GuestResource,
}

impl SandboxObservationKind {
    pub const COUNT: usize = 2;

    pub const fn index(self) -> usize {
        match self {
            Self::ProcessIo => 0,
            Self::GuestResource => 1,
        }
    }

    pub const fn of(observation: &Observation) -> Self {
        match observation {
            Observation::ProcessIo(_) => Self::ProcessIo,
            Observation::GuestResource(_) => Self::GuestResource,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SandboxObservationDescriptor {
    observation_index: u32,
    kind: SandboxObservationKind,
}

impl SandboxObservationDescriptor {
    pub const fn new(observation_index: u32, kind: SandboxObservationKind) -> Self {
        Self {
            observation_index,
            kind,
        }
    }

    pub const fn observation_index(self) -> u32 {
        self.observation_index
    }

    pub const fn kind(self) -> SandboxObservationKind {
        self.kind
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SandboxObservationDescriptors {
    descriptors: Box<[SandboxObservationDescriptor]>,
}

impl SandboxObservationDescriptors {
    pub fn from_batch(batch: &ObservationBatch) -> Result<Self, &'static str> {
        let mut descriptors = Vec::with_capacity(batch.observations.len());
        for (index, observation) in batch.observations.iter().enumerate() {
            let index =
                u32::try_from(index).map_err(|_| "sandbox observation count exceeds u32")?;
            descriptors.push(SandboxObservationDescriptor::new(
                index,
                SandboxObservationKind::of(observation),
            ));
        }
        Ok(Self {
            descriptors: descriptors.into_boxed_slice(),
        })
    }

    pub fn as_slice(&self) -> &[SandboxObservationDescriptor] {
        &self.descriptors
    }

    pub fn len(&self) -> usize {
        self.descriptors.len()
    }

    pub fn is_empty(&self) -> bool {
        self.descriptors.is_empty()
    }
}
