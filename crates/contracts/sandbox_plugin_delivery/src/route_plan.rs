#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SandboxConsumerId(u64);

impl SandboxConsumerId {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SandboxRegistryGeneration(u64);

impl SandboxRegistryGeneration {
    pub const INITIAL: Self = Self(0);

    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SandboxConsumerRoute {
    consumer_id: SandboxConsumerId,
    observation_indices: Box<[u32]>,
}

impl SandboxConsumerRoute {
    pub fn new(consumer_id: SandboxConsumerId, observation_indices: Box<[u32]>) -> Self {
        Self {
            consumer_id,
            observation_indices,
        }
    }

    pub const fn consumer_id(&self) -> SandboxConsumerId {
        self.consumer_id
    }

    pub fn observation_indices(&self) -> &[u32] {
        &self.observation_indices
    }

    pub fn into_parts(self) -> (SandboxConsumerId, Box<[u32]>) {
        (self.consumer_id, self.observation_indices)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SandboxRoutePlan {
    generation: SandboxRegistryGeneration,
    observation_count: u32,
    routes: Box<[SandboxConsumerRoute]>,
    unmatched_indices: Box<[u32]>,
}

impl SandboxRoutePlan {
    pub fn new(
        generation: SandboxRegistryGeneration,
        observation_count: u32,
        routes: Box<[SandboxConsumerRoute]>,
        unmatched_indices: Box<[u32]>,
    ) -> Self {
        Self {
            generation,
            observation_count,
            routes,
            unmatched_indices,
        }
    }

    pub const fn generation(&self) -> SandboxRegistryGeneration {
        self.generation
    }

    pub const fn observation_count(&self) -> u32 {
        self.observation_count
    }

    pub fn routes(&self) -> &[SandboxConsumerRoute] {
        &self.routes
    }

    pub fn unmatched_indices(&self) -> &[u32] {
        &self.unmatched_indices
    }

    pub fn into_parts(
        self,
    ) -> (
        SandboxRegistryGeneration,
        u32,
        Box<[SandboxConsumerRoute]>,
        Box<[u32]>,
    ) {
        (
            self.generation,
            self.observation_count,
            self.routes,
            self.unmatched_indices,
        )
    }
}
