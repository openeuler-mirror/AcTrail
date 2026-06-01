//! Application protocol analyzers over retained plaintext payload segments.

use config_core::daemon::ApplicationProtocolConfig;
use model_core::event::ApplicationPayload;
use model_core::payload::{PayloadContentState, PayloadSegment, PayloadSourceBoundary};

#[path = "application_protocol/http1.rs"]
mod http1;
#[path = "application_protocol/http2.rs"]
mod http2;

pub(super) const COLLECTOR_NAME: &str = "application-protocol-analyzer";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ApplicationEventDraft {
    pub payload: ApplicationPayload,
}

pub(super) struct ApplicationProtocolAnalyzer {
    config: ApplicationProtocolConfig,
    http1: http1::Http1Analyzer,
    http2: http2::Http2Analyzer,
}

impl ApplicationProtocolAnalyzer {
    pub(super) fn new(config: ApplicationProtocolConfig) -> Self {
        Self {
            http1: http1::Http1Analyzer::new(config.clone()),
            http2: http2::Http2Analyzer::new(config.clone()),
            config,
        }
    }

    pub(super) fn analyze(
        &mut self,
        segment: &PayloadSegment,
    ) -> Result<Vec<ApplicationEventDraft>, String> {
        if !self.config.enabled
            || segment.content_state != PayloadContentState::Plaintext
            || !matches!(
                segment.source_boundary,
                PayloadSourceBoundary::TlsUserSpace | PayloadSourceBoundary::Syscall
            )
        {
            return Ok(Vec::new());
        }

        let mut drafts = Vec::new();
        if self.config.http1_enabled {
            drafts.extend(self.http1.analyze(segment)?);
        }
        if self.config.http2_enabled {
            drafts.extend(self.http2.analyze(segment)?);
        }
        Ok(drafts)
    }
}
