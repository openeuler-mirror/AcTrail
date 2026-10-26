//! The connection incarnation owns each stream's directional message lifecycle.
use crate::llm_pipeline::assembly::http2::Http2BodyAssembly;
use crate::llm_pipeline::assembly::router::LiveStreamDirection;
use crate::llm_pipeline::transport::http2::Http2Decoder;
use model_core::payload::PayloadStreamIdentity;
use std::collections::BTreeMap;

#[derive(Clone, Eq, PartialEq)]
pub(super) struct ConnectionKey(pub(super) PayloadStreamIdentity);
impl Ord for ConnectionKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (
            self.0.trace_id,
            self.0.process,
            self.0.source_boundary as u8,
            &self.0.stream_key,
        )
            .cmp(&(
                other.0.trace_id,
                other.0.process,
                other.0.source_boundary as u8,
                &other.0.stream_key,
            ))
    }
}
impl PartialOrd for ConnectionKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
#[derive(Default)]
pub(super) struct StreamLifecycle {
    pub(super) started: [bool; 2],
    pub(super) ended: [bool; 2],
}
pub(super) struct Direction {
    pub(super) decoder: Http2Decoder,
    pub(super) body: Http2BodyAssembly,
}
pub(super) struct Connection {
    pub(super) prefix: String,
    pub(super) observed_at: std::time::SystemTime,
    pub(super) directions: BTreeMap<LiveStreamDirection, Direction>,
    pub(super) streams: BTreeMap<u32, StreamLifecycle>,
    pub(super) highest_request: u32,
    pub(super) completed: Vec<u32>,
}
impl Connection {
    pub(super) fn message_id(&self, stream_id: u32, direction: LiveStreamDirection) -> String {
        format!(
            "{}:stream:{}:{}",
            self.prefix,
            stream_id,
            Self::direction_name(direction)
        )
    }
    pub(super) fn direction_name(direction: LiveStreamDirection) -> &'static str {
        match direction {
            LiveStreamDirection::Outbound => "request",
            LiveStreamDirection::Inbound => "response",
        }
    }
    pub(super) fn index(direction: LiveStreamDirection) -> usize {
        match direction {
            LiveStreamDirection::Outbound => 0,
            LiveStreamDirection::Inbound => 1,
        }
    }
    pub(super) fn direction(&mut self, direction: LiveStreamDirection) -> &mut Direction {
        self.directions
            .entry(direction)
            .or_insert_with(|| Direction {
                decoder: Http2Decoder::new(format!(
                    "{}:{}",
                    self.prefix,
                    Self::direction_name(direction)
                )),
                body: Http2BodyAssembly::default(),
            })
    }
}
