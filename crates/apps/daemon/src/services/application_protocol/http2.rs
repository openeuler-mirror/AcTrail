//! Application retention projection of frames decoded by the shared transport owner.
use super::{ApplicationEventDraft, base64_encode};
use config_core::daemon::{
    ApplicationProtocolConfig, Http2DataContentRetention, SemanticRetentionConfig,
};
use model_core::event::{ApplicationBody, ApplicationPayload};
use semantic_action_runtime::Http2FrameObservation;
use std::collections::BTreeMap;

pub(super) struct Http2Observer;
impl Http2Observer {
    pub(super) fn observe(
        frames: Vec<Http2FrameObservation>,
        config: &ApplicationProtocolConfig,
        retention: &SemanticRetentionConfig,
        summary_only: bool,
    ) -> Result<Vec<ApplicationEventDraft>, String> {
        let mut drafts = Vec::new();
        for frame in frames {
            let mut append = |payload| {
                drafts.push(
                    ApplicationEventDraft::complete(payload)
                        .at(frame.observed_at)
                        .for_process(frame.source.process),
                );
            };
            if frame.connection_preface {
                if retention.http2_frame_summary_enabled() {
                    append(Self::preface_payload(&frame));
                }
            } else if !(summary_only && frame.frame_type == 0) {
                if retention.http2_frame_summary_enabled() {
                    append(Self::frame_payload(&frame));
                }
                if frame.frame_type == 0
                    && (retention.http2_frame_summary_enabled()
                        || !matches!(
                            retention.http2_data_content(),
                            Http2DataContentRetention::None
                        ))
                {
                    append(Self::data_payload(
                        &frame,
                        config,
                        retention.http2_data_content(),
                    )?);
                }
            }
        }
        Ok(drafts)
    }
    fn preface_payload(frame: &Http2FrameObservation) -> ApplicationPayload {
        ApplicationPayload {
            protocol: "h2".to_string(),
            operation: "connection_preface".to_string(),
            summary: "client connection preface".to_string(),
            body: None,
            metadata: Self::base_metadata(frame, None),
        }
    }

    fn frame_payload(frame: &Http2FrameObservation) -> ApplicationPayload {
        let mut metadata = Self::base_metadata(frame, Some(frame.stream_id));
        Self::insert_frame_metadata(&mut metadata, frame);
        ApplicationPayload {
            protocol: "h2".to_string(),
            operation: "frame".to_string(),
            summary: format!(
                "{} stream={} len={}",
                Self::frame_type_name(frame.frame_type),
                frame.stream_id,
                frame.payload.len()
            ),
            body: None,
            metadata,
        }
    }

    fn data_payload(
        frame: &Http2FrameObservation,
        config: &ApplicationProtocolConfig,
        content: Http2DataContentRetention,
    ) -> Result<ApplicationPayload, String> {
        let mut metadata = Self::base_metadata(frame, Some(frame.stream_id));
        Self::insert_frame_metadata(&mut metadata, frame);
        metadata.insert("data_size".to_string(), frame.payload.len().to_string());
        let mut body = None;
        match content {
            Http2DataContentRetention::None => {}
            Http2DataContentRetention::Preview if config.http2_emit_data_preview => {
                match Self::preview_data(&frame.payload, config.http2_max_data_preview_bytes)? {
                    Some((preview, truncated)) => {
                        metadata.insert("data_preview".to_string(), preview);
                        metadata
                            .insert("data_preview_truncated".to_string(), truncated.to_string());
                    }
                    None => {
                        metadata.insert("data_preview_omitted".to_string(), "non_utf8".to_string());
                    }
                }
            }
            Http2DataContentRetention::Preview => {}
            Http2DataContentRetention::Raw => {
                body = Some(ApplicationBody::Base64(base64_encode(&frame.payload)));
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
            body,
            metadata,
        })
    }

    fn base_metadata(
        frame: &Http2FrameObservation,
        stream_id: Option<u32>,
    ) -> BTreeMap<String, String> {
        let mut metadata = BTreeMap::from([
            (
                "direction".to_string(),
                format!("{:?}", frame.source.direction).to_lowercase(),
            ),
            (
                "source_boundary".to_string(),
                format!("{:?}", frame.source.source_boundary),
            ),
            (
                "stream_key".to_string(),
                frame.source.stream_key.to_string(),
            ),
            (
                "payload_sequence".to_string(),
                frame.source.payload_sequence.to_string(),
            ),
            (
                "payload_segment_id".to_string(),
                frame.source.segment_id.get().to_string(),
            ),
        ]);
        metadata.insert("http2.frame_id".to_string(), frame.frame_id.clone());
        if let Some(id) = &frame.message_id {
            metadata.insert("http2.message_id".to_string(), id.clone());
        }
        if let Some(stream_id) = stream_id {
            metadata.insert("stream_id".to_string(), stream_id.to_string());
        }
        metadata
    }

    fn insert_frame_metadata(
        metadata: &mut BTreeMap<String, String>,
        frame: &Http2FrameObservation,
    ) {
        metadata.insert(
            "frame_type".to_string(),
            Self::frame_type_name(frame.frame_type).to_string(),
        );
        metadata.insert("frame_type_id".to_string(), frame.frame_type.to_string());
        metadata.insert("flags".to_string(), format!("0x{:02x}", frame.flags));
        metadata.insert("length".to_string(), frame.payload.len().to_string());
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
    fn frame_type_name(frame_type: u8) -> &'static str {
        match frame_type {
            0x0 => "DATA",
            0x1 => "HEADERS",
            0x2 => "PRIORITY",
            0x3 => "RST_STREAM",
            0x4 => "SETTINGS",
            0x5 => "PUSH_PROMISE",
            0x6 => "PING",
            0x7 => "GOAWAY",
            0x8 => "WINDOW_UPDATE",
            0x9 => "CONTINUATION",
            _ => "UNKNOWN",
        }
    }
}
