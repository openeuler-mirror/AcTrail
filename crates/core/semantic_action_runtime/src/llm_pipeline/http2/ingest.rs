//! Frames establish stream ownership before their DATA reaches content decoders.
use super::protocol::Http2Protocol;
use super::state::{Connection, StreamLifecycle};
use crate::llm_pipeline::assembly::router::{LiveStreamDirection, PayloadStreamGroupKey};
use crate::llm_pipeline::projection::ProjectionBatch;
use crate::llm_pipeline::provider::codec::LlmCodecRegistry;
use crate::llm_pipeline::stream::finalizer::StreamFinalizationReason;
use crate::llm_pipeline::transport::http2::{Http2Event, data_payload};
use crate::{Http1AbortReason, Http1MessageEnd, Http1MessageEvent};
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
                Http2Event::BufferCapacityExceeded => {
                    return (
                        terminal,
                        Some(StreamFinalizationReason::BufferBytesExceeded),
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
            let header = match connection.direction(direction).headers.advance(&frame) {
                Ok(header) => header,
                Err(failure) => {
                    tracing::warn!(
                        ?failure,
                        stream_id = frame.stream_id,
                        "HTTP/2 header decoding failed"
                    );
                    if failure.connection_failed() {
                        output.http2_frames.push(frame);
                        return (
                            terminal,
                            Some(StreamFinalizationReason::ProtocolDecodeFailed),
                            false,
                        );
                    }
                    self.reset_stream(
                        connection,
                        frame.stream_id,
                        frame.observed_at,
                        StreamFinalizationReason::ProtocolDecodeFailed,
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
            };
            if frame.connection_preface || frame.stream_id == 0 {
                output.http2_frames.push(frame);
                continue;
            }
            if header.as_ref().is_some_and(|block| block.push_promise) {
                output.http2_frames.push(frame);
                continue;
            }
            let stream_id = frame.stream_id;
            if matches!(frame.frame_type, 1 | 9) && header.is_none() {
                frame.message_id = Some(connection.message_id(stream_id, direction));
                output.http2_frames.push(frame);
                continue;
            }
            if header
                .as_ref()
                .and_then(|block| block.head.status_code.as_deref())
                .is_some_and(|status| status.starts_with('1'))
            {
                output.http2_frames.push(frame);
                continue;
            }
            let creates_source = header.is_some() || frame.frame_type == 0;
            if !connection.streams.contains_key(&stream_id)
                && (creates_source || frame.frame_type == 3)
            {
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
                if first && header.is_none() {
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
                    output.http2_frames.push(frame);
                    clean = false;
                    continue;
                }
                state.started[index] = true;
                if direction == LiveStreamDirection::Outbound {
                    connection.highest_request = connection.highest_request.max(stream_id);
                }
                let id = connection.message_id(stream_id, direction);
                let ended = header
                    .as_ref()
                    .map_or(frame.frame_type == 0 && frame.flags & 1 != 0, |block| {
                        block.end_stream
                    });
                if first {
                    let Some(mut head) = header.map(|block| block.head) else {
                        continue;
                    };
                    head.message_id = id.clone();
                    let mut http = self.facts.head(&head, output);
                    http.attributes
                        .insert("stream_id".into(), stream_id.to_string());
                    http.attributes
                        .insert("http.headers_decoded".into(), "true".into());
                    http.evidence = vec![semantic_action::SemanticEvidence {
                        kind: semantic_action::SemanticEvidenceKind::PayloadSegment,
                        id: head.source.segment_id.get(),
                        role: "http.message.payload".into(),
                    }];
                    if let Some(fact) = self.facts.messages.get_mut(&id) {
                        fact.http = http.clone();
                    }
                    output.actions.push(http);
                    if config.llm_layer_enabled() {
                        let failure = connection.direction(direction).body.begin_message(
                            stream_id,
                            &head,
                            self.limits,
                        );
                        if let Some(failure) = failure {
                            self.fail_content(
                                connection,
                                stream_id,
                                &id,
                                failure,
                                frame.observed_at,
                                direction,
                                group,
                                config,
                                codecs,
                                output,
                            );
                            clean = false;
                        }
                    }
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
                            output.http2_frames.push(frame);
                            clean = false;
                            continue;
                        };
                        if config.llm_layer_enabled() {
                            let failure = connection.direction(direction).body.append_data(
                                stream_id,
                                &id,
                                data,
                                &evidence,
                                self.limits,
                            );
                            if let Some(failure) = failure {
                                self.fail_content(
                                    connection,
                                    stream_id,
                                    &id,
                                    failure,
                                    frame.observed_at,
                                    direction,
                                    group,
                                    config,
                                    codecs,
                                    output,
                                );
                                clean = false;
                            }
                        }
                    }
                    if ended {
                        if config.llm_layer_enabled() {
                            let failure = connection.direction(direction).body.end_stream(
                                stream_id,
                                frame.observed_at,
                                self.limits,
                            );
                            if let Some(failure) = failure {
                                self.fail_content(
                                    connection,
                                    stream_id,
                                    &id,
                                    failure,
                                    frame.observed_at,
                                    direction,
                                    group,
                                    config,
                                    codecs,
                                    output,
                                );
                                clean = false;
                            }
                        }
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
                    connection
                        .direction(direction)
                        .body
                        .discard_stream(stream_id);
                    clean = false;
                    if ended {
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

    fn fail_content(
        &mut self,
        connection: &mut Connection,
        stream_id: u32,
        id: &str,
        failure: crate::llm_pipeline::assembly::router::AssemblyResetReason,
        at: std::time::SystemTime,
        direction: LiveStreamDirection,
        group: &PayloadStreamGroupKey,
        config: &SemanticRetentionConfig,
        codecs: &LlmCodecRegistry,
        output: &mut ProjectionBatch,
    ) {
        tracing::warn!(
            ?failure,
            stream_id,
            ?direction,
            "HTTP/2 content decoding stopped"
        );
        let reason = failure.finalization_reason();
        Self::diagnose(reason, group, at, Some(stream_id), output);
        let projected = connection
            .direction(direction)
            .body
            .discard_consumer(stream_id, id, reason, at, direction, config, codecs, group);
        self.consume(
            projected,
            output,
            config.l0_llm_call.context_compact.enabled,
        );
        self.facts.finalize_content(id, reason, at, output);
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
                self.consume(
                    projected,
                    output,
                    config.l0_llm_call.context_compact.enabled,
                );
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
