//! Construction and extension inputs accepted by the pipeline facade.
use super::pipeline::LiveLlmProjector;
use crate::llm_pipeline::assembly::router::AssemblyLimits;
use crate::llm_pipeline::projection::ProjectionCoordinator;
use crate::llm_pipeline::provider::codec::{
    LlmCodecPlugin, LlmCodecPluginStatus, LlmCodecRegistry,
};
use crate::llm_pipeline::transport::websocket::WebSocketTransport;
use crate::llm_pipeline::websocket::WebSocketProtocol;
use config_core::daemon::SemanticRetentionConfig;
impl LiveLlmProjector {
    pub(crate) fn new(config: SemanticRetentionConfig) -> Self {
        Self {
            http1: crate::llm_pipeline::http1::Http1Protocol::new(&config),
            http2: crate::llm_pipeline::http2::Http2Protocol::new(&config),
            websocket: WebSocketTransport::new(&config),
            websocket_protocol: WebSocketProtocol::new(&config),
            protocol_router: super::router::ProtocolRouter::new(
                AssemblyLimits::from(config.l0_llm_call.assembly),
                config.l2_http.max_active_directions,
            ),
            projection: ProjectionCoordinator::new(&config),
            config,
            codecs: LlmCodecRegistry::default(),
        }
    }
    pub(crate) fn register_codec(
        &mut self,
        plugin: std::sync::Arc<dyn LlmCodecPlugin>,
    ) -> Result<(), String> {
        self.codecs.register(plugin)
    }
    pub(crate) fn unregister_codec(&mut self, id: &str) -> bool {
        self.codecs.unregister(id)
    }
    pub(crate) fn codec_statuses(&self) -> Vec<LlmCodecPluginStatus> {
        self.codecs.statuses()
    }
}
