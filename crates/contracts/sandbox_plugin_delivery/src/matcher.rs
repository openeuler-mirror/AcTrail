use crate::{SandboxIntentQueryError, SandboxIntentQueryResult, SandboxObservationDescriptor};

pub trait SandboxPluginIntentMatcher: Send + Sync {
    fn query_intent(
        &self,
        descriptors: &[SandboxObservationDescriptor],
    ) -> Result<SandboxIntentQueryResult, SandboxIntentQueryError>;
}
