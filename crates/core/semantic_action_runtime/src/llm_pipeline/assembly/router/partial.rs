use std::time::SystemTime;

use config_core::daemon::SemanticRetentionConfig;

use crate::llm_pipeline::projection::ProjectionBatch;
use crate::llm_pipeline::provider::codec::LlmCodecRegistry;
use crate::llm_pipeline::stream::finalizer::StreamFinalizationReason;

use super::PayloadStreamGroupKey;
use super::router::StreamBody;

impl StreamBody {
    pub(super) fn materialize_incomplete_requests(
        &mut self,
        config: &SemanticRetentionConfig,
        codecs: &LlmCodecRegistry,
        key: &PayloadStreamGroupKey,
        reason: StreamFinalizationReason,
        finished_at: SystemTime,
    ) -> ProjectionBatch {
        match self {
            Self::Plain(plain) => {
                plain.materialize_incomplete_request(config, codecs, key, reason, finished_at)
            }
            Self::Http2(http2) => {
                http2.materialize_incomplete_requests(config, codecs, key, reason, finished_at)
            }
        }
    }
}
