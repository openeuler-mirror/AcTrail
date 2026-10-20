//! Frames establish stream ownership before their DATA reaches content decoders.
use super::protocol::Http2Protocol;
use super::state::{Connection, StreamLifecycle};
use crate::llm_pipeline::assembly::router::{LiveStreamDirection, PayloadStreamGroupKey};
use crate::llm_pipeline::projection::ProjectionBatch;
use crate::llm_pipeline::provider::codec::LlmCodecRegistry;
use crate::llm_pipeline::stream::finalizer::StreamFinalizationReason;
use crate::llm_pipeline::transport::http2::{Http2Event, data_payload};
use crate::{Http1AbortReason, Http1MessageEnd, Http1MessageEvent, Http1MessageHead};
use config_core::daemon::SemanticRetentionConfig;

impl Http2Protocol {
    pub(super) fn observe_frames(
        &mut self,
        events: Vec<Http2Event>,
        connection: &mut Connection,
        direction: LiveStreamDirection,
        group: &PayloadStreamGroupKey,
        config: &SemanticRetentionConfig,
        codecs: &LlmCodecRegistry,
        output: &mut ProjectionBatch,
    ) -> (
        Vec<Http1MessageEvent>,
        Option<StreamFinalizationReason>,
        bool,
    ) {
        let mut terminal = Vec::new();
        let mut clean = true;
        let index = Connection::index(direction);
        for event in events {
            let (mut frame, evidence) = match event {
                Http2Event::Frame {
                    observation,
                    evidence,
                } => (observation, evidence),
                Http2Event::ConnectionFailure => {
                    return (
                        terminal,
                        Some(StreamFinalizationReason::ProtocolDecodeFailed),
                        false,
                    );
                }
                Http2Event::StreamFailure {
                    stream_id,
                    observed_at,
                } => {
                    self.reset_stream(
                        connection,
                        stream_id,
                        observed_at,
                        StreamFinalizationReason::ProtocolDecodeFailed,
                        group,
                        config,
                        codecs,
                        output,
                        &mut terminal,
                    );
                    clean = false;
                    continue;
                }
            };
            if frame.connection_preface || frame.stream_id == 0 {
                output.http2_frames.push(frame);
                continue;
            }
            let stream_id = frame.stream_id;
            let creates_source = matches!(frame.frame_type, 0 | 1);
            if !connection.streams.contains_key(&stream_id)
                && (creates_source || frame.frame_type == 3)
            {
                // Requests open monotonically numbered streams. A removed stream
                // below the request watermark cannot become a new incarnation.
                if stream_id <= connection.highest_request {
                    output.http2_frames.push(frame);
                    clean = false;
                    continue;
                }
                if self
                    .stream_counts
                    .get(&group.trace_id)
                    .copied()
                    .unwrap_or(0)
                    >= self.max_streams
                {
                    return (
                        terminal,
                        Some(StreamFinalizationReason::Http2ActiveStreamCapacity),
                        false,
                    );
                }
                connection
                    .streams
                    .insert(stream_id, StreamLifecycle::default());
                *self.stream_counts.entry(group.trace_id).or_default() += 1;
            }
            if frame.frame_type == 3 {
                self.reset_stream(
                    connection,
                    stream_id,
                    frame.observed_at,
                    if frame.payload.len() == 4 {
                        StreamFinalizationReason::Http2StreamReset
                    } else {
                        StreamFinalizationReason::ProtocolDecodeFailed
                    },
                    group,
                    config,
                    codecs,
                    output,
                    &mut terminal,
                );
                output.http2_frames.push(frame);
                clean = false;
                continue;
            }
            if creates_source {
                let Some(state) = connection.streams.get_mut(&stream_id) else {
                    output.http2_frames.push(frame);
                    continue;
                };
                if state.ended[index] {
                    clean = false;
                    output.http2_frames.push(frame);
                    continue;
                }
                let first = !state.started[index];
                state.started[index] = true;
                if direction == LiveStreamDirection::Outbound {
                    connection.highest_request = connection.highest_request.max(stream_id);
                }
                let id = connection.message_id(stream_id, direction);
                if first {
                    let head = Http1MessageHead {
                        message_id: id.clone(),
                        source: frame.source.clone(),
                        protocol: "h2",
                        method: None,
                        authority: None,
                        path: None,
                        status_code: None,
                        reason: None,
                        headers_text: String::new(),
                        declared_body_len: None,
                    };
                    let mut http = self.facts.head(&head, output);
                    http.title = format!(
                        "HTTP/2 {} stream {}",
                        Connection::direction_name(direction),
                        stream_id
                    );
                    http.attributes
                        .insert("stream_id".into(), stream_id.to_string());
                    http.attributes
                        .insert("http.headers_decoded".into(), "false".into());
                    http.evidence = frame
                        .payload_segment_ids
                        .iter()
                        .map(|id| semantic_action::SemanticEvidence {
                            kind: semantic_action::SemanticEvidenceKind::PayloadSegment,
                            id: id.get(),
                            role: "http.message.payload".into(),
                        })
                        .collect();
                    if let Some(fact) = self.facts.messages.get_mut(&id) {
                        fact.http = http.clone();
                    }
                    output.actions.push(http);
                    let request = connection.message_id(stream_id, LiveStreamDirection::Outbound);
                    let response = connection.message_id(stream_id, LiveStreamDirection::Inbound);
                    if self.facts.messages.contains_key(&request)
                        && self.facts.messages.contains_key(&response)
                    {
                        self.facts.bind_stream(&request, &response, output);
                    }
                }
                if let Some(fact) = self.facts.messages.get_mut(&id) {
                    fact.last_observed = frame.observed_at;
                    frame.message_id = Some(id.clone());
                    if frame.frame_type == 0 {
                        if connection
                            .direction(direction)
                            .body
                            .consumer_discarded(stream_id)
                        {
                            clean = false;
                        }
                        let Some(data) = data_payload(frame.flags, &frame.payload) else {
                            self.reset_stream(
                                connection,
                                stream_id,
                                frame.observed_at,
                                StreamFinalizationReason::ProtocolDecodeFailed,
                                group,
                                config,
                                codecs,
                                output,
                                &mut terminal,
                            );
                            clean = false;
                            output.http2_frames.push(frame);
                            continue;
                        };
                        if config.llm_layer_enabled()
                            && let Some(reason) = connection.direction(direction).body.append_data(
                                stream_id,
                                &id,
                                data,
                                &evidence,
                                self.limits,
                            )
                        {
                            let reason = reason.finalization_reason();
                            Self::diagnose(
                                reason,
                                group,
                                frame.observed_at,
                                Some(stream_id),
                                output,
                            );
                            let projected = connection.direction(direction).body.discard_consumer(
                                stream_id,
                                &id,
                                reason,
                                frame.observed_at,
                                direction,
                                config,
                                codecs,
                                group,
                            );
                            self.consume(projected, output);
                            self.facts
                                .finalize_content(&id, reason, frame.observed_at, output);
                            clean = false;
                        }
                    }
                    if frame.flags & 1 != 0 {
                        connection
                            .direction(direction)
                            .body
                            .end_stream(stream_id, frame.observed_at);
                        if let Some(state) = connection.streams.get_mut(&stream_id) {
                            state.ended[index] = true;
                            if state.ended.iter().all(|ended| *ended) {
                                connection.completed.push(stream_id);
                            }
                        }
                        terminal.push(Http1MessageEvent::End(Http1MessageEnd {
                            message_id: id,
                            observed_at: frame.observed_at,
                        }));
                    }
                } else {
                    // Fact eviction permanently stops this source's semantic consumer.
                    connection
                        .direction(direction)
                        .body
                        .discard_stream(stream_id);
                    clean = false;
                    if frame.flags & 1 != 0 {
                        if let Some(state) = connection.streams.get_mut(&stream_id) {
                            state.ended[index] = true;
                            if state.ended.iter().all(|ended| *ended) {
                                connection.completed.push(stream_id);
                            }
                        }
                    }
                }
            }
            output.http2_frames.push(frame);
        }
        (terminal, None, clean)
    }

    fn reset_stream(
        &mut self,
        connection: &mut Connection,
        stream_id: u32,
        at: std::time::SystemTime,
        reason: StreamFinalizationReason,
        group: &PayloadStreamGroupKey,
        config: &SemanticRetentionConfig,
        codecs: &LlmCodecRegistry,
        output: &mut ProjectionBatch,
        terminal: &mut Vec<Http1MessageEvent>,
    ) {
        let already_ended = connection
            .streams
            .get(&stream_id)
            .map_or([false, false], |state| state.ended);
        if already_ended == [true, true] {
            return;
        }
        Self::diagnose(reason, group, at, Some(stream_id), output);
        if let Some(state) = connection.streams.get_mut(&stream_id) {
            state.ended = [true, true];
            // Unknown response-first resets remain as bounded tombstones until
            // the request watermark establishes that their id cannot reopen.
            if stream_id <= connection.highest_request {
                connection.completed.push(stream_id);
            }
        }
        for direction in [LiveStreamDirection::Outbound, LiveStreamDirection::Inbound] {
            if already_ended[Connection::index(direction)] {
                continue;
            }
            if let Some(state) = connection.directions.get_mut(&direction) {
                let projected = state
                    .body
                    .abort_stream(stream_id, reason, at, direction, config, codecs, group);
                self.consume(projected, output);
            }
            terminal.push(Http1MessageEvent::Abort {
                message_id: connection.message_id(stream_id, direction),
                reason: match reason {
                    StreamFinalizationReason::Http2StreamReset => {
                        Http1AbortReason::Http2StreamReset
                    }
                    StreamFinalizationReason::BufferBytesExceeded
                    | StreamFinalizationReason::SegmentRangesExceeded => {
                        Http1AbortReason::CapacityExceeded
                    }
                    _ => Http1AbortReason::DecodeFailure,
                },
                observed_at: at,
            });
        }
    }
}
