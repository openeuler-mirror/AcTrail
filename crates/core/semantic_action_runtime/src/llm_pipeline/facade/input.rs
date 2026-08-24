//! Construction and extension inputs accepted by the pipeline facade.

use std::collections::BTreeMap;

use config_core::daemon::SemanticRetentionConfig;

use crate::llm_pipeline::assembly::router::{AssemblyLimits, LiveStreamState};
use crate::llm_pipeline::projection::ProjectionCoordinator;
use crate::llm_pipeline::provider::codec::{
    LlmCodecPlugin, LlmCodecPluginStatus, LlmCodecRegistry,
};
use crate::llm_pipeline::transport::websocket;

use super::pipeline::LiveLlmProjector;

impl LiveLlmProjector {
    pub(crate) fn new(config: SemanticRetentionConfig) -> Self {
        let websocket_response_max_bytes =
            usize::try_from(config.l0_llm_call.assembly.max_buffer_bytes)
                .expect("validated LLM assembly byte limit must fit usize");
        let websocket = websocket::WebSocketLlmAdapter::new(
            config.l0_llm_call.websocket_max_connections_per_process,
            websocket_response_max_bytes,
        );
        let assembly_limits = AssemblyLimits::from(config.l0_llm_call.assembly);
        let max_confirmed_http_exchanges_per_stream =
            usize::try_from(config.l2_http.exchange.max_pending_responses_per_stream)
                .expect("validated HTTP exchange response limit must fit usize");
        let projection =
            ProjectionCoordinator::new(&config, max_confirmed_http_exchanges_per_stream);
        Self {
            config,
            codecs: LlmCodecRegistry::default(),
            streams: BTreeMap::<_, LiveStreamState>::new(),
            projection,
            assembly_limits,
            websocket,
            websocket_stream_ownership: Default::default(),
        }
    }

    pub(crate) fn register_codec(
        &mut self,
        plugin: std::sync::Arc<dyn LlmCodecPlugin>,
    ) -> Result<(), String> {
        self.codecs.register(plugin)
    }

    pub(crate) fn unregister_codec(&mut self, instance_id: &str) -> bool {
        self.codecs.unregister(instance_id)
    }

    pub(crate) fn codec_statuses(&self) -> Vec<LlmCodecPluginStatus> {
        self.codecs.statuses()
    }
}
