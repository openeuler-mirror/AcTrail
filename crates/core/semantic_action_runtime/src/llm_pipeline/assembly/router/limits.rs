//! Validated assembly limits and fail-local reset reasons.

use config_core::daemon::LlmAssemblyConfig;

use crate::llm_pipeline::stream::finalizer::StreamFinalizationReason;

#[derive(Clone, Copy)]
pub(in crate::llm_pipeline) struct AssemblyLimits {
    pub(in crate::llm_pipeline) max_buffer_bytes: usize,
    pub(in crate::llm_pipeline) max_segment_ranges: usize,
}

impl From<LlmAssemblyConfig> for AssemblyLimits {
    fn from(config: LlmAssemblyConfig) -> Self {
        Self {
            max_buffer_bytes: usize::try_from(config.max_buffer_bytes)
                .expect("validated LLM assembly byte limit must fit usize"),
            max_segment_ranges: usize::try_from(config.max_segment_ranges)
                .expect("validated LLM assembly segment limit must fit usize"),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(in crate::llm_pipeline) enum AssemblyResetReason {
    BufferBytesExceeded,
    SegmentRangesExceeded,
    ContentDecodeFailed,
}

impl AssemblyResetReason {
    pub(in crate::llm_pipeline) fn finalization_reason(self) -> StreamFinalizationReason {
        match self {
            Self::BufferBytesExceeded => StreamFinalizationReason::BufferBytesExceeded,
            Self::SegmentRangesExceeded => StreamFinalizationReason::SegmentRangesExceeded,
            Self::ContentDecodeFailed => StreamFinalizationReason::ProtocolDecodeFailed,
        }
    }
}
