#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::llm_pipeline) enum ContentDecodeFailure {
    BufferCapacity,
    InvalidContentEncoding,
}
