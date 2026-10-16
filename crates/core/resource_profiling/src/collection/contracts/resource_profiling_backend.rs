use super::SamplingContext;

/// An instance owns the baseline for its bound target. Input is a transient
/// snapshot supplied by the authoritative identity owner.
pub trait ResourceProfilingBackend {
    type Input: ?Sized;
    type Sample;

    fn sample(
        &mut self,
        input: &Self::Input,
        context: SamplingContext,
    ) -> Result<Option<Self::Sample>, String>;
}
