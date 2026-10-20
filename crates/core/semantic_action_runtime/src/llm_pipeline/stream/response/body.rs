//! LLM response body and SSE framing adapter.

use semantic_action::{
    LlmJsonResponseInput, LlmParsedResponse, LlmProviderResponseStreamParser, LlmResponseRetention,
    LlmResponseTermination, LlmSseEvent as ProviderSseEvent, LlmSseResponseInput, LlmTokenUsage,
};
use serde_json::Value;

use crate::llm_pipeline::config::StreamClassifierConfig;

use crate::llm_pipeline::provider::codec::{LlmCodecRegistry, NormalizedSseEvent, SseCodecEvent};
use crate::llm_pipeline::provider::{
    extract_token_usage, new_sse_stream_parser, parse_json_response, tool_calls_json,
};
use crate::llm_pipeline::stream::classifier::StreamClassifier;
use crate::llm_pipeline::stream::sse_framer::{CompleteSseEvent, IncrementalSseFramer};

const SSE_DONE_MARKER: &str = "[DONE]";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::llm_pipeline) struct LlmResponseBody {
    pub(in crate::llm_pipeline) provider_id: String,
    pub(in crate::llm_pipeline) provider_response_id: Option<String>,
    pub(in crate::llm_pipeline) json_valid: bool,
    pub(in crate::llm_pipeline) model: Option<String>,
    pub(in crate::llm_pipeline) content_text: Option<String>,
    pub(in crate::llm_pipeline) reasoning_text: Option<String>,
    pub(in crate::llm_pipeline) tool_calls_json: Option<String>,
    pub(in crate::llm_pipeline) token_usage: Option<LlmTokenUsage>,
    pub(in crate::llm_pipeline) chunk_count: usize,
    pub(in crate::llm_pipeline) termination: Option<LlmResponseTermination>,
    pub(in crate::llm_pipeline) stream: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::llm_pipeline) struct ProviderStreamUpdate {
    pub(in crate::llm_pipeline) termination: Option<LlmResponseTermination>,
    pub(in crate::llm_pipeline) stream: bool,
    pub(in crate::llm_pipeline) chunk_count: usize,
}

pub(in crate::llm_pipeline) fn parse_llm_response_body(
    body: &[u8],
    codecs: &LlmCodecRegistry,
    retention: LlmResponseRetention,
) -> Option<LlmResponseBody> {
    LlmResponseBodyParser { codecs, retention }.parse(body)
}

/// Byte-source of the body passed to the incremental SSE parser.
///
/// The incremental cache is only valid while the body bytes are a strict
/// prefix extension of what was previously parsed. Switching projection
/// paths changes the byte source, so the cache is reseeded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::llm_pipeline) enum SseBodySource {
    SplitHttp,
}

/// Incremental SSE parse state for one in-flight response message.
///
/// Every payload segment re-ran `parse_llm_response_body` over the whole
/// accumulated SSE body, which is O(M²) over a stream of M events. This cache
/// parses only newly appended events through a provider stream parser and
/// accumulates the aggregate response body instead.
pub(crate) struct IncrementalSseCache {
    retention: LlmResponseRetention,
    source: SseBodySource,
    codec_revision: u64,
    framer: IncrementalSseFramer,
    classifier: StreamClassifier,
    stream_parser: Option<Box<dyn LlmProviderResponseStreamParser + Send>>,
    provider_id_override: Option<String>,
    provider_response_id: Option<String>,
    token_usage: Option<LlmTokenUsage>,
    termination: Option<LlmResponseTermination>,
    chunk_count: usize,
}

impl IncrementalSseCache {
    pub(in crate::llm_pipeline) fn is_confirmed_llm(&self) -> bool {
        self.classifier.is_confirmed_llm()
    }

    fn seed(
        source: SseBodySource,
        body: &[u8],
        codecs: &LlmCodecRegistry,
        classifier_config: StreamClassifierConfig,
        retention: LlmResponseRetention,
    ) -> Option<(Self, Option<ProviderStreamUpdate>)> {
        let mut framer = IncrementalSseFramer::default();
        let complete_events = framer.advance(body).ok()?;
        if complete_events.is_empty() && !looks_like_sse_prefix(body) {
            return None;
        }
        let mut normalization_failed = false;
        let normalized = complete_events
            .into_iter()
            .map(|event| normalized_event(codecs, event))
            .map_while(|event| {
                normalization_failed = event.is_none();
                event
            })
            .collect::<Vec<_>>();
        let mut classifier = StreamClassifier::new(classifier_config);
        let initial_window = normalized
            .iter()
            .filter(|event| classifier.belongs_to_initial_window(event.end_offset))
            .collect::<Vec<_>>();
        let mut stream_parser = select_stream_parser(&initial_window, retention);
        if stream_parser.is_none() {
            stream_parser = normalized
                .iter()
                .filter(|event| !classifier_config.can_sniff_through(event.end_offset))
                .find_map(|event| select_stream_parser(&[event], retention));
        }
        if stream_parser.is_some() {
            classifier.confirm_llm();
        }
        let mut cache = Self {
            retention,
            source,
            codec_revision: codecs.revision(),
            framer,
            classifier,
            stream_parser,
            provider_id_override: None,
            provider_response_id: None,
            token_usage: None,
            termination: None,
            chunk_count: 0,
        };
        if cache.classifier.is_confirmed_llm() {
            for event in normalized {
                cache.apply_event(event)?;
                if cache.termination.is_some() {
                    break;
                }
            }
        }
        if normalization_failed && cache.termination.is_none() {
            return None;
        }
        let progress = cache.progress();
        Some((cache, progress))
    }

    fn apply_event(&mut self, event: IncrementalNormalizedEvent) -> Option<()> {
        if self.termination.is_some() {
            return Some(());
        }
        let parsed = self
            .stream_parser
            .as_mut()?
            .observe_event(provider_normalized_sse_event(&event.event));
        self.provider_id_override = self.provider_id_override.take().or(event.provider_id);
        if let Some(response_id) = event.event.json.as_ref().and_then(|value| {
            value
                .get("response")
                .and_then(provider_response_id)
                .or_else(|| provider_response_id(value))
        }) {
            self.provider_response_id = Some(response_id);
        }
        if let Some(usage) = event
            .event
            .json
            .as_ref()
            .filter(|_| self.retention.usage)
            .and_then(extract_token_usage)
        {
            self.token_usage = Some(usage);
        }
        self.termination = LlmResponseTermination::combine(self.termination, parsed.termination);
        if parsed.text_chunk_count > 0 {
            self.chunk_count += 1;
        }
        Some(())
    }

    fn observe_delta(
        &mut self,
        body: &[u8],
        codecs: &LlmCodecRegistry,
    ) -> Result<Option<ProviderStreamUpdate>, ()> {
        if self.termination.is_some() {
            return Ok(self.progress());
        }
        if self.codec_revision != codecs.revision() {
            return Err(());
        }
        let events = self.framer.advance(body).map_err(|_| ())?;
        if self.classifier.is_confirmed_llm() {
            for event in events {
                let event = normalized_event(codecs, event).ok_or(())?;
                self.apply_event(event).ok_or(())?;
                if self.termination.is_some() {
                    break;
                }
            }
            return Ok(self.progress());
        }
        for event in events {
            let event = normalized_event(codecs, event).ok_or(())?;
            self.classifier.belongs_to_initial_window(event.end_offset);
            if let Some(parser) = select_stream_parser(&[&event], self.retention) {
                self.stream_parser = Some(parser);
                self.classifier.confirm_llm();
                self.replay_retained_body(body, codecs)?;
                return Ok(self.progress());
            }
        }
        Ok(None)
    }

    fn replay_retained_body(&mut self, body: &[u8], codecs: &LlmCodecRegistry) -> Result<(), ()> {
        self.provider_id_override = None;
        self.provider_response_id = None;
        self.token_usage = None;
        self.termination = None;
        self.chunk_count = 0;
        let mut replay = IncrementalSseFramer::default();
        for event in replay.advance(body).map_err(|_| ())? {
            let event = normalized_event(codecs, event).ok_or(())?;
            self.apply_event(event).ok_or(())?;
            if self.termination.is_some() {
                break;
            }
        }
        Ok(())
    }

    pub(in crate::llm_pipeline) fn body(&mut self) -> Option<LlmResponseBody> {
        let mut parsed = self.stream_parser.as_mut()?.finish()?;
        if let Some(usage) = self.token_usage.clone() {
            parsed.token_usage = Some(usage);
        }
        let mut body = response_body(
            false,
            parsed,
            self.provider_response_id.clone(),
            self.retention,
        );
        if let Some(provider_id) = self.provider_id_override.clone() {
            body.provider_id = provider_id;
        }
        Some(body)
    }

    fn progress(&self) -> Option<ProviderStreamUpdate> {
        self.classifier
            .is_confirmed_llm()
            .then_some(ProviderStreamUpdate {
                termination: self.termination,
                stream: true,
                chunk_count: self.chunk_count,
            })
    }
}

fn advance_cache(
    source: SseBodySource,
    body: &[u8],
    codecs: &LlmCodecRegistry,
    classifier_config: StreamClassifierConfig,
    retention: LlmResponseRetention,
    cache: &mut Option<IncrementalSseCache>,
    allow_batch: bool,
) -> CacheAdvance {
    if let Some(existing) = cache.as_ref() {
        if existing.source != source || existing.retention != retention {
            *cache = None;
        }
    }
    match cache {
        Some(existing) => match existing.observe_delta(body, codecs) {
            Ok(Some(progress)) => CacheAdvance::Incremental(progress),
            Ok(None) => CacheAdvance::Unparseable,
            Err(()) => {
                *cache = None;
                if allow_batch {
                    batch_advance(body, codecs, retention)
                } else {
                    CacheAdvance::Unparseable
                }
            }
        },
        None => {
            if let Some((seeded, progress)) =
                IncrementalSseCache::seed(source, body, codecs, classifier_config, retention)
            {
                *cache = Some(seeded);
                progress.map_or(CacheAdvance::Unparseable, CacheAdvance::Incremental)
            } else {
                if allow_batch {
                    batch_advance(body, codecs, retention)
                } else {
                    CacheAdvance::Unparseable
                }
            }
        }
    }
}

struct IncrementalNormalizedEvent {
    event: NormalizedSseEvent,
    provider_id: Option<String>,
    end_offset: usize,
}

fn normalized_event(
    codecs: &LlmCodecRegistry,
    event: CompleteSseEvent,
) -> Option<IncrementalNormalizedEvent> {
    let end_offset = event.end_offset;
    let event = sse_codec_event(event);
    if let Some(decoded) = codecs.decode_sse_event(&event) {
        let data = String::from_utf8(decoded.body).ok()?;
        let trimmed = data.trim();
        return Some(IncrementalNormalizedEvent {
            event: NormalizedSseEvent {
                index: event.index,
                event_type: event.event_type,
                id: event.id,
                json: serde_json::from_str::<Value>(trimmed).ok(),
                done_marker: trimmed == SSE_DONE_MARKER,
                data,
            },
            provider_id: decoded.provider_id,
            end_offset,
        });
    }
    Some(IncrementalNormalizedEvent {
        event: NormalizedSseEvent {
            index: event.index,
            event_type: event.event_type,
            id: event.id,
            data: event.data,
            json: event.json,
            done_marker: event.done_marker,
        },
        provider_id: None,
        end_offset,
    })
}

fn select_stream_parser(
    events: &[&IncrementalNormalizedEvent],
    retention: LlmResponseRetention,
) -> Option<Box<dyn LlmProviderResponseStreamParser + Send>> {
    if events.is_empty() {
        return None;
    }
    let provider_events = events
        .iter()
        .map(|event| provider_normalized_sse_event(&event.event))
        .collect::<Vec<_>>();
    new_sse_stream_parser(
        LlmSseResponseInput {
            // Built-in stream classifiers operate on complete normalized events.
            // Keeping the accumulated text out of this hot path avoids validating
            // or materializing the complete body after every network append.
            text: "",
            events: &provider_events,
        },
        retention,
    )
    .map(|(parser, _provider_id)| parser)
}

fn looks_like_sse_prefix(body: &[u8]) -> bool {
    let start = body
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(body.len());
    let end = body[start..]
        .iter()
        .position(|byte| *byte == b'\n' || *byte == b'\r')
        .map_or(body.len(), |offset| start + offset);
    let prefix = &body[start..end];
    !prefix.is_empty()
        && [b"data:".as_slice(), b"event:", b"id:", b"retry:", b":"]
            .iter()
            .any(|candidate| candidate.starts_with(&prefix) || prefix.starts_with(candidate))
}

enum CacheAdvance {
    Incremental(ProviderStreamUpdate),
    Batch(LlmResponseBody),
    Unparseable,
}

fn batch_advance(
    body: &[u8],
    codecs: &LlmCodecRegistry,
    retention: LlmResponseRetention,
) -> CacheAdvance {
    match parse_llm_response_body(body, codecs, retention) {
        Some(parsed) => CacheAdvance::Batch(parsed),
        None => CacheAdvance::Unparseable,
    }
}

/// Advance the incremental SSE cache and report progress scalars only.
///
/// Unlike [`parse_llm_response_body_incremental`], this does not materialize
/// the accumulated response body, so in-flight chunks avoid cloning the
/// accumulated content and tool calls.
pub(in crate::llm_pipeline) fn parse_llm_response_progress(
    source: SseBodySource,
    body: &[u8],
    codecs: &LlmCodecRegistry,
    classifier_config: StreamClassifierConfig,
    retention: LlmResponseRetention,
    cache: &mut Option<IncrementalSseCache>,
    allow_batch: bool,
) -> Option<ProviderStreamUpdate> {
    match advance_cache(
        source,
        body,
        codecs,
        classifier_config,
        retention,
        cache,
        allow_batch,
    ) {
        CacheAdvance::Incremental(progress) => Some(progress),
        CacheAdvance::Batch(parsed) => Some(ProviderStreamUpdate {
            termination: parsed.termination,
            stream: parsed.stream,
            chunk_count: parsed.chunk_count,
        }),
        CacheAdvance::Unparseable => None,
    }
}

/// Parse an SSE response body incrementally when possible, falling back to the
/// batch parser for codec paths, unsupported providers, or mid-event merges.
pub(in crate::llm_pipeline) fn parse_llm_response_body_incremental(
    source: SseBodySource,
    body: &[u8],
    codecs: &LlmCodecRegistry,
    classifier_config: StreamClassifierConfig,
    retention: LlmResponseRetention,
    cache: &mut Option<IncrementalSseCache>,
) -> Option<LlmResponseBody> {
    match advance_cache(
        source,
        body,
        codecs,
        classifier_config,
        retention,
        cache,
        true,
    ) {
        CacheAdvance::Incremental(_) => cache.as_mut()?.body(),
        CacheAdvance::Batch(parsed) => Some(parsed),
        CacheAdvance::Unparseable => None,
    }
}

struct LlmResponseBodyParser<'a> {
    codecs: &'a LlmCodecRegistry,
    retention: LlmResponseRetention,
}

impl LlmResponseBodyParser<'_> {
    fn parse(&self, body: &[u8]) -> Option<LlmResponseBody> {
        let text = String::from_utf8_lossy(body);
        if let Some(sse) = self.parse_sse_response_body(&text) {
            return Some(sse);
        }
        let json = serde_json::from_slice::<Value>(body).ok();
        let value = json.as_ref()?;
        let parsed = parse_json_response(
            LlmJsonResponseInput {
                text: &text,
                json: value,
            },
            self.retention,
        )?;
        Some(response_body(
            true,
            parsed,
            provider_response_id(value),
            self.retention,
        ))
    }

    fn parse_sse_response_body(&self, text: &str) -> Option<LlmResponseBody> {
        let mut framer = IncrementalSseFramer::default();
        let mut normalization_failed = false;
        let events = framer
            .advance(text.as_bytes())
            .ok()?
            .into_iter()
            .map(|event| normalized_event(self.codecs, event))
            .map_while(|event| {
                normalization_failed = event.is_none();
                event
            })
            .collect::<Vec<_>>();
        let mut parser = select_stream_parser(&events.iter().collect::<Vec<_>>(), self.retention)?;
        let mut consumed = 0;
        let mut terminated = false;
        for event in &events {
            consumed += 1;
            if parser
                .observe_event(provider_normalized_sse_event(&event.event))
                .termination
                .is_some()
            {
                terminated = true;
                break;
            }
        }
        if normalization_failed && !terminated {
            return None;
        }
        // Batch recovery obeys the same provider boundary as incremental parsing.
        // Metadata must come from that consumed prefix too.
        let events = &events[..consumed];
        let mut parsed = parser.finish()?;
        let response_id = events.iter().rev().find_map(|event| {
            event.event.json.as_ref().and_then(|value| {
                value
                    .get("response")
                    .and_then(provider_response_id)
                    .or_else(|| provider_response_id(value))
            })
        });
        if self.retention.usage {
            if let Some(usage) = events
                .iter()
                .rev()
                .find_map(|event| event.event.json.as_ref().and_then(extract_token_usage))
            {
                parsed.token_usage = Some(usage);
            }
        }
        let mut body = response_body(false, parsed, response_id, self.retention);
        if let Some(provider_id) = events.iter().find_map(|event| event.provider_id.as_ref()) {
            body.provider_id = provider_id.clone();
        }
        Some(body)
    }
}

fn response_body(
    json_valid: bool,
    parsed: LlmParsedResponse,
    provider_response_id: Option<String>,
    retention: LlmResponseRetention,
) -> LlmResponseBody {
    let tool_calls_json = retention
        .tool_calls
        .then(|| tool_calls_json(&parsed.tool_calls))
        .flatten();
    LlmResponseBody {
        provider_id: parsed.provider_id.to_string(),
        provider_response_id,
        json_valid,
        model: parsed.model,
        content_text: parsed.content_text,
        reasoning_text: parsed.reasoning_text,
        tool_calls_json,
        token_usage: parsed.token_usage,
        chunk_count: parsed.chunk_count,
        termination: parsed.termination,
        stream: parsed.stream,
    }
}

fn provider_response_id(value: &Value) -> Option<String> {
    value
        .get("id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn sse_codec_event(event: CompleteSseEvent) -> SseCodecEvent {
    raw_sse_event(event.index, event.event_type, event.id, event.data)
}

fn raw_sse_event(
    index: usize,
    event_type: Option<String>,
    id: Option<String>,
    data: String,
) -> SseCodecEvent {
    let trimmed = data.trim();
    SseCodecEvent {
        index,
        event_type,
        id,
        json: serde_json::from_str::<Value>(trimmed).ok(),
        done_marker: trimmed == SSE_DONE_MARKER,
        data,
    }
}

fn provider_normalized_sse_event(event: &NormalizedSseEvent) -> ProviderSseEvent<'_> {
    ProviderSseEvent {
        index: event.index,
        event_type: event.event_type.as_deref(),
        id: event.id.as_deref(),
        data: &event.data,
        json: event.json.as_ref(),
        done_marker: event.done_marker,
    }
}
