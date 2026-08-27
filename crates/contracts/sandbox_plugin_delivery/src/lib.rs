//! Dedicated delivery boundary between Gateway Ingest and sandbox plugins.

mod descriptor;
mod matcher;
mod publisher;
mod result;
mod route_plan;
mod source;

pub use descriptor::{
    SandboxObservationDescriptor, SandboxObservationDescriptors, SandboxObservationKind,
};
pub use matcher::SandboxPluginIntentMatcher;
pub use publisher::{
    SandboxConsumerBatch, SandboxObservationConsumer, SandboxPluginPublisher, SandboxPublishBatch,
};
pub use result::{
    SandboxConsumeError, SandboxConsumeReport, SandboxConsumerDelivery, SandboxDeliveryOutcome,
    SandboxIntentQueryError, SandboxIntentQueryResult, SandboxPublishError, SandboxPublishReport,
};
pub use route_plan::{
    SandboxConsumerId, SandboxConsumerRoute, SandboxRegistryGeneration, SandboxRoutePlan,
};
pub use source::{SandboxSource, SandboxSourceError};
