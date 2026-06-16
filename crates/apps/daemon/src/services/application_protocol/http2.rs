//! HTTP/2 frame analyzer over retained plaintext TLS payload segments.

use std::collections::{BTreeMap, btree_map::Entry};

use config_core::daemon::{
    ApplicationProtocolConfig, Http2DataContentRetention, SemanticRetentionConfig,
};
use model_core::event::ApplicationPayload;
use model_core::ids::TraceId;
use model_core::payload::{PayloadDirection, PayloadSegment, PayloadStreamKey};
use model_core::process::ProcessIdentity;

use super::ApplicationEventDraft;
use super::base64_encode;

#[path = "http2/frame.rs"]
mod frame;

pub(super) struct Http2Analyzer {
    #[cfg(test)]
    config: ApplicationProtocolConfig,
    #[cfg(test)]
    semantic_retention: SemanticRetentionConfig,
    connections: BTreeMap<ConnectionKey, ConnectionState>,
}

impl Http2Analyzer {
    pub(super) fn new(config: ApplicationProtocolConfig) -> Self {
        let _ = &config;
        Self {
            #[cfg(test)]
            config,
            #[cfg(test)]
            semantic_retention: SemanticRetentionConfig::default(),
            connections: BTreeMap::new(),
        }
    }

    #[cfg(test)]
    pub(super) fn new_with_retention(
        config: ApplicationProtocolConfig,
        semantic_retention: SemanticRetentionConfig,
    ) -> Self {
        Self {
            config,
            semantic_retention,
            connections: BTreeMap::new(),
        }
    }

    #[cfg(test)]
    pub(super) fn analyze(
        &mut self,
        segment: &PayloadSegment,
    ) -> Result<Vec<ApplicationEventDraft>, String> {
        let config = self.config.clone();
        let semantic_retention = self.semantic_retention.clone();
        self.analyze_with_config(segment, &config, &semantic_retention, false)
    }

    pub(super) fn analyze_with_config(
        &mut self,
        segment: &PayloadSegment,
        config: &ApplicationProtocolConfig,
        semantic_retention: &SemanticRetentionConfig,
        summary_only: bool,
    ) -> Result<Vec<ApplicationEventDraft>, String> {
        let key = connection_key(segment);
        let state = match self.connections.entry(key.clone()) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry)
                if starts_or_can_be_frame(segment, config.http2_max_frame_bytes) =>
            {
                entry.insert(ConnectionState::default())
            }
            Entry::Vacant(_) => return Ok(Vec::new()),
        };
        if state.append(segment, config).is_err() {
            state.clear_direction(segment.direction);
            if state.is_idle() {
                self.connections.remove(&key);
            }
            return Ok(Vec::new());
        }
        let drafts = state.drain_events(segment, config, semantic_retention, summary_only)?;
        if state.is_idle() {
            self.connections.remove(&key);
        }
        Ok(drafts)
    }

    pub(super) fn forget_trace(&mut self, trace_id: TraceId) {
        self.connections.retain(|key, _| key.trace_id != trace_id);
    }

    #[cfg(test)]
    pub(super) fn connection_count(&self) -> usize {
        self.connections.len()
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ConnectionKey {
    trace_id: TraceId,
    process: ProcessIdentity,
    stream_key: PayloadStreamKey,
}

#[derive(Default)]
struct ConnectionState {
    h2_confirmed: bool,
    preface_emitted: bool,
    outbound: DirectionState,
    inbound: DirectionState,
}

impl ConnectionState {
    fn append(
        &mut self,
        segment: &PayloadSegment,
        config: &ApplicationProtocolConfig,
    ) -> Result<(), String> {
        self.direction_mut(segment.direction)
            .append(&segment.bytes, config.http2_max_connection_buffer_bytes)
    }

    fn drain_events(
        &mut self,
        segment: &PayloadSegment,
        config: &ApplicationProtocolConfig,
        semantic_retention: &SemanticRetentionConfig,
        summary_only: bool,
    ) -> Result<Vec<ApplicationEventDraft>, String> {
        let mut drafts = Vec::new();
        self.confirm_preface(segment, semantic_retention, &mut drafts)?;
        if !self.h2_confirmed && !self.direction(segment.direction).can_parse_frame() {
            return Ok(Vec::new());
        }

        let mut clear_direction = false;
        let direction = self.direction_mut(segment.direction);
        loop {
            let status = match frame::decode_next(&direction.buffer, config.http2_max_frame_bytes) {
                Ok(status) => status,
                Err(_) if starts_with_known_frame_header(&direction.buffer) => {
                    clear_direction = true;
                    break;
                }
                Err(_) => {
                    let Some(offset) = find_next_plausible_frame_offset(
                        &direction.buffer,
                        config.http2_max_frame_bytes,
                    ) else {
                        direction.buffer.clear();
                        break;
                    };
                    direction.buffer.drain(..offset);
                    continue;
                }
            };
            let frame = match status {
                frame::DecodeStatus::NeedMore => break,
                frame::DecodeStatus::Frame(frame) => frame,
            };
            let consumed = frame::encoded_len(&frame);
            if summary_only && frame.frame_type == frame::DATA_FRAME_TYPE {
                direction.buffer.drain(..consumed);
                continue;
            }
            if semantic_retention.http2_frame_summary_enabled() {
                drafts.push(ApplicationEventDraft {
                    payload: frame_payload(segment, &frame),
                });
            }
            if frame.frame_type == frame::DATA_FRAME_TYPE {
                let data_content = semantic_retention.http2_data_content();
                if semantic_retention.http2_frame_summary_enabled()
                    || !matches!(data_content, Http2DataContentRetention::None)
                {
                    drafts.push(ApplicationEventDraft {
                        payload: data_payload(segment, &frame, config, data_content)?,
                    });
                }
            }
            direction.buffer.drain(..consumed);
        }
        if clear_direction {
            self.clear_direction(segment.direction);
        }
        Ok(drafts)
    }

    fn confirm_preface(
        &mut self,
        segment: &PayloadSegment,
        semantic_retention: &SemanticRetentionConfig,
        drafts: &mut Vec<ApplicationEventDraft>,
    ) -> Result<(), String> {
        if self.h2_confirmed {
            return Ok(());
        }
        let direction = self.direction_mut(segment.direction);
        if direction.buffer.len() < frame::CONNECTION_PREFACE.len() {
            return Ok(());
        }
        if !direction.buffer.starts_with(frame::CONNECTION_PREFACE) {
            return Ok(());
        }
        direction.buffer.drain(..frame::CONNECTION_PREFACE.len());
        self.h2_confirmed = true;
        if !self.preface_emitted && semantic_retention.http2_frame_summary_enabled() {
            self.preface_emitted = true;
            drafts.push(ApplicationEventDraft {
                payload: preface_payload(segment),
            });
        }
        Ok(())
    }

    fn direction(&self, direction: PayloadDirection) -> &DirectionState {
        match direction {
            PayloadDirection::Outbound => &self.outbound,
            PayloadDirection::Inbound => &self.inbound,
        }
    }

    fn direction_mut(&mut self, direction: PayloadDirection) -> &mut DirectionState {
        match direction {
            PayloadDirection::Outbound => &mut self.outbound,
            PayloadDirection::Inbound => &mut self.inbound,
        }
    }

    fn is_idle(&self) -> bool {
        !self.h2_confirmed && self.outbound.buffer.is_empty() && self.inbound.buffer.is_empty()
    }

    fn clear_direction(&mut self, direction: PayloadDirection) {
        self.direction_mut(direction).buffer.clear();
    }
}

#[derive(Default)]
struct DirectionState {
    buffer: Vec<u8>,
}

impl DirectionState {
    fn append(&mut self, bytes: &[u8], max_buffer_bytes: u64) -> Result<(), String> {
        let next_len = self
            .buffer
            .len()
            .checked_add(bytes.len())
            .ok_or_else(|| "HTTP/2 connection buffer size overflow".to_string())?;
        if u64::try_from(next_len).map_err(|error| error.to_string())? > max_buffer_bytes {
            return Err(format!(
                "HTTP/2 connection buffer would exceed configured maximum {max_buffer_bytes} bytes"
            ));
        }
        self.buffer.extend_from_slice(bytes);
        Ok(())
    }

    fn can_parse_frame(&self) -> bool {
        self.buffer.len() >= frame::FRAME_HEADER_BYTES
    }
}

fn starts_or_can_be_frame(segment: &PayloadSegment, max_frame_bytes: u64) -> bool {
    segment.bytes.starts_with(frame::CONNECTION_PREFACE)
        || protocol_hint_is_h2(segment)
        || starts_with_plausible_frame(&segment.bytes, max_frame_bytes)
}

fn starts_with_plausible_frame(bytes: &[u8], max_frame_bytes: u64) -> bool {
    if bytes.len() < frame::FRAME_HEADER_BYTES {
        return false;
    }
    let length =
        (usize::from(bytes[0]) << 16) | (usize::from(bytes[1]) << 8) | usize::from(bytes[2]);
    if u64::try_from(length).map_or(true, |length| length > max_frame_bytes) {
        return false;
    }
    starts_with_known_frame_header(bytes)
}

fn find_next_plausible_frame_offset(bytes: &[u8], max_frame_bytes: u64) -> Option<usize> {
    if bytes.len() <= frame::FRAME_HEADER_BYTES {
        return None;
    }
    (1..=bytes.len() - frame::FRAME_HEADER_BYTES)
        .find(|offset| starts_with_plausible_frame(&bytes[*offset..], max_frame_bytes))
}

fn starts_with_known_frame_header(bytes: &[u8]) -> bool {
    if bytes.len() < frame::FRAME_HEADER_BYTES {
        return false;
    }
    let frame_type = bytes[3];
    let stream_id = (u32::from(bytes[5] & 0x7f) << 24)
        | (u32::from(bytes[6]) << 16)
        | (u32::from(bytes[7]) << 8)
        | u32::from(bytes[8]);
    match frame_type {
        0x0 | 0x1 | 0x2 | 0x3 | 0x5 | 0x9 => stream_id != 0,
        0x4 | 0x6 | 0x7 => stream_id == 0,
        0x8 => true,
        _ => false,
    }
}

fn protocol_hint_is_h2(segment: &PayloadSegment) -> bool {
    segment
        .protocol_hint
        .as_deref()
        .map(|hint| {
            hint.eq_ignore_ascii_case("h2")
                || hint.eq_ignore_ascii_case("http2")
                || hint.eq_ignore_ascii_case("http/2")
                || hint.eq_ignore_ascii_case("http/2.0")
        })
        .unwrap_or(false)
}

fn preface_payload(segment: &PayloadSegment) -> ApplicationPayload {
    ApplicationPayload {
        protocol: "h2".to_string(),
        operation: "connection_preface".to_string(),
        summary: "client connection preface".to_string(),
        metadata: base_metadata(segment, None),
    }
}

fn frame_payload(segment: &PayloadSegment, frame: &frame::Frame) -> ApplicationPayload {
    let mut metadata = base_metadata(segment, Some(frame.stream_id));
    insert_frame_metadata(&mut metadata, frame);
    ApplicationPayload {
        protocol: "h2".to_string(),
        operation: "frame".to_string(),
        summary: format!(
            "{} stream={} len={}",
            frame.type_name(),
            frame.stream_id,
            frame.length
        ),
        metadata,
    }
}

fn data_payload(
    segment: &PayloadSegment,
    frame: &frame::Frame,
    config: &ApplicationProtocolConfig,
    content: Http2DataContentRetention,
) -> Result<ApplicationPayload, String> {
    let mut metadata = base_metadata(segment, Some(frame.stream_id));
    insert_frame_metadata(&mut metadata, frame);
    metadata.insert("data_size".to_string(), frame.payload.len().to_string());
    match content {
        Http2DataContentRetention::None => {}
        Http2DataContentRetention::Preview if config.http2_emit_data_preview => {
            match preview_data(&frame.payload, config.http2_max_data_preview_bytes)? {
                Some((preview, truncated)) => {
                    metadata.insert("data_preview".to_string(), preview);
                    metadata.insert("data_preview_truncated".to_string(), truncated.to_string());
                }
                None => {
                    metadata.insert("data_preview_omitted".to_string(), "non_utf8".to_string());
                }
            }
        }
        Http2DataContentRetention::Preview => {}
        Http2DataContentRetention::Raw => {
            metadata.insert("data_base64".to_string(), base64_encode(&frame.payload));
        }
    }
    Ok(ApplicationPayload {
        protocol: "h2".to_string(),
        operation: "data".to_string(),
        summary: format!(
            "DATA stream={} len={}",
            frame.stream_id,
            frame.payload.len()
        ),
        metadata,
    })
}

fn base_metadata(segment: &PayloadSegment, stream_id: Option<u32>) -> BTreeMap<String, String> {
    let mut metadata = BTreeMap::from([
        (
            "direction".to_string(),
            format!("{:?}", segment.direction).to_lowercase(),
        ),
        (
            "source_boundary".to_string(),
            format!("{:?}", segment.source_boundary),
        ),
        ("stream_key".to_string(), segment.stream_key.to_string()),
        ("payload_sequence".to_string(), segment.sequence.to_string()),
        (
            "payload_segment_id".to_string(),
            segment.segment_id.get().to_string(),
        ),
    ]);
    if let Some(stream_id) = stream_id {
        metadata.insert("stream_id".to_string(), stream_id.to_string());
    }
    metadata
}

fn insert_frame_metadata(metadata: &mut BTreeMap<String, String>, frame: &frame::Frame) {
    metadata.insert("frame_type".to_string(), frame.type_name().to_string());
    metadata.insert("frame_type_id".to_string(), frame.frame_type.to_string());
    metadata.insert("flags".to_string(), frame.flags_hex());
    metadata.insert("length".to_string(), frame.length.to_string());
}

fn connection_key(segment: &PayloadSegment) -> ConnectionKey {
    ConnectionKey {
        trace_id: segment.trace_id,
        process: segment.process.clone(),
        stream_key: segment.stream_key.clone(),
    }
}

fn preview_data(bytes: &[u8], max_bytes: u64) -> Result<Option<(String, bool)>, String> {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return Ok(None);
    };
    let max_bytes = usize::try_from(max_bytes).map_err(|error| error.to_string())?;
    if text.len() <= max_bytes {
        return Ok(Some((text.to_string(), false)));
    }
    let mut end = max_bytes;
    while !text.is_char_boundary(end) {
        end = end
            .checked_sub(1)
            .ok_or_else(|| "HTTP/2 data preview boundary underflow".to_string())?;
    }
    Ok(Some((text[..end].to_string(), true)))
}

#[cfg(test)]
#[path = "http2/tests.rs"]
mod tests;
