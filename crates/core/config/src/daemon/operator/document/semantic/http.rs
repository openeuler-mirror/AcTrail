use super::*;
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(in crate::daemon::operator::document) struct Http1ProtocolDocument {
    pub max_buffer_bytes: u64,
    pub max_evidence_ranges: u32,
}
impl Default for Http1ProtocolDocument {
    fn default() -> Self {
        Self::from_config(Http1ProtocolConfig::default())
    }
}
impl Http1ProtocolDocument {
    pub(super) fn from_config(config: Http1ProtocolConfig) -> Self {
        Self {
            max_buffer_bytes: config.max_buffer_bytes,
            max_evidence_ranges: config.max_evidence_ranges,
        }
    }
    pub(super) fn to_config(&self) -> Result<Http1ProtocolConfig, String> {
        Ok(Http1ProtocolConfig {
            max_buffer_bytes: require_positive_u64(
                "semantic_retention.l2_http.http1.max_buffer_bytes",
                self.max_buffer_bytes,
            )?,
            max_evidence_ranges: require_positive_u32(
                "semantic_retention.l2_http.http1.max_evidence_ranges",
                self.max_evidence_ranges,
            )?,
        })
    }
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(in crate::daemon::operator::document) struct Http2ProtocolDocument {
    pub max_active_streams_per_trace: u32,
    pub max_evidence_ranges: u32,
    pub max_frame_bytes: u64,
    pub max_connection_buffer_bytes: u64,
}
impl Default for Http2ProtocolDocument {
    fn default() -> Self {
        Self::from_config(Http2ProtocolConfig::default())
    }
}
impl Http2ProtocolDocument {
    pub(super) fn from_config(config: Http2ProtocolConfig) -> Self {
        Self {
            max_active_streams_per_trace: config.max_active_streams_per_trace,
            max_evidence_ranges: config.max_evidence_ranges,
            max_frame_bytes: config.max_frame_bytes,
            max_connection_buffer_bytes: config.max_connection_buffer_bytes,
        }
    }
    pub(super) fn to_config(&self) -> Result<Http2ProtocolConfig, String> {
        Ok(Http2ProtocolConfig {
            max_evidence_ranges: require_positive_u32(
                "semantic_retention.l2_http.http2.max_evidence_ranges",
                self.max_evidence_ranges,
            )?,
            max_active_streams_per_trace: require_positive_u32(
                "semantic_retention.l2_http.http2.max_active_streams_per_trace",
                self.max_active_streams_per_trace,
            )?,
            max_frame_bytes: require_positive_u64(
                "semantic_retention.l2_http.http2.max_frame_bytes",
                self.max_frame_bytes,
            )?,
            max_connection_buffer_bytes: require_positive_u64(
                "semantic_retention.l2_http.http2.max_connection_buffer_bytes",
                self.max_connection_buffer_bytes,
            )?,
        })
    }
}
