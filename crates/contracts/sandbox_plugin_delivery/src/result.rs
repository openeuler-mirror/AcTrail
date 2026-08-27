use crate::{SandboxConsumerId, SandboxRegistryGeneration, SandboxRoutePlan};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SandboxIntentQueryResult {
    NoInterest {
        generation: SandboxRegistryGeneration,
        observation_count: u32,
    },
    Matched(SandboxRoutePlan),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SandboxIntentQueryError {
    ObservationCountOverflow,
    InvalidDescriptorIndex { index: u32 },
    DuplicateDescriptorIndex { index: u32 },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SandboxConsumeReport {
    pub observed_records: u64,
    pub dropped_records: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SandboxConsumeError {
    pub code: String,
    pub message: String,
}

impl SandboxConsumeError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SandboxDeliveryOutcome {
    Accepted { observation_count: u32 },
    Full { observation_count: u32 },
    Closed { observation_count: u32 },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SandboxConsumerDelivery {
    pub consumer_id: SandboxConsumerId,
    pub outcome: SandboxDeliveryOutcome,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SandboxPublishReport {
    pub deliveries: Vec<SandboxConsumerDelivery>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SandboxPublishError {
    ObservationCountOverflow,
    ExpiredPlan {
        plan_generation: SandboxRegistryGeneration,
        current_generation: SandboxRegistryGeneration,
    },
    ObservationCountMismatch {
        planned: u32,
        actual: u32,
    },
    EmptyRoutePlan,
    EmptyConsumerRoute {
        consumer_id: SandboxConsumerId,
    },
    InvalidObservationIndex {
        consumer_id: SandboxConsumerId,
        index: u32,
    },
    DuplicateObservationIndex {
        consumer_id: SandboxConsumerId,
        index: u32,
    },
    SelectorMismatch {
        consumer_id: SandboxConsumerId,
        index: u32,
    },
    DuplicateConsumerRoute {
        consumer_id: SandboxConsumerId,
    },
    MissingConsumer {
        consumer_id: SandboxConsumerId,
    },
    InvalidUnmatchedIndex {
        index: u32,
    },
    DuplicateUnmatchedIndex {
        index: u32,
    },
    RoutedAndUnmatched {
        index: u32,
    },
    UnassignedObservation {
        index: u32,
    },
}
