//! LLM response body and SSE framing adapter.

use semantic_action::{
    LlmJsonResponseInput, LlmParsedResponse, LlmProviderResponseStreamParser,
    LlmSseEvent as ProviderSseEvent, LlmSseResponseInput, LlmTokenUsage,
};
use serde_json::Value;

use crate::llm_pipeline::config::StreamClassifierConfig;

use crate::llm_pipeline::provider::codec::{LlmCodecRegistry, NormalizedSseEvent, SseCodecEvent};
use crate::llm_pipeline::provider::{
    extract_token_usage, new_sse_stream_parser, parse_json_response, parse_sse_response,
    tool_calls_json,
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
    pub(in crate::llm_pipeline) done: bool,
    pub(in crate::llm_pipeline) stream: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::llm_pipeline) struct ProviderStreamUpdate {
    pub(in crate::llm_pipeline) done: bool,
    pub(in crate::llm_pipeline) stream: bool,
    pub(in crate::llm_pipeline) chunk_count: usize,
}

pub(in crate::llm_pipeline) fn parse_llm_response_body(
    body: &[u8],
    codecs: &LlmCodecRegistry,
) -> Option<LlmResponseBody> {
    LlmResponseBodyParser { codecs }.parse(body)
}

/// Byte-source of the body passed to the incremental SSE parser.
///
/// The incremental cache is only valid while the body bytes are a strict
/// prefix extension of what was previously parsed. Switching projection
/// paths changes the byte source, so the cache is reseeded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::llm_pipeline) enum SseBodySource {
    SplitHttp,
    RawBytes,
}

/// Incremental SSE parse state for one in-flight response message.
///
/// Every payload segment re-ran `parse_llm_response_body` over the whole
/// accumulated SSE body, which is O(M²) over a stream of M events. This cache
/// parses only newly appended events through a provider stream parser and
/// accumulates the aggregate response body instead.
pub(crate) struct IncrementalSseCache {
    source: SseBodySource,
    codec_revision: u64,
    framer: IncrementalSseFramer,
    classifier: StreamClassifier,
    stream_parser: Option<Box<dyn LlmProviderResponseStreamParser + Send>>,
    provider_id_override: Option<String>,
    provider_response_id: Option<String>,
    token_usage: Option<LlmTokenUsage>,
    done: bool,
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
    ) -> Option<(Self, Option<ProviderStreamUpdate>)> {
        let mut framer = IncrementalSseFramer::default();
        let complete_events = framer.advance(body).ok()?;
        if complete_events.is_empty() && !looks_like_sse_prefix(body) {
            return None;
        }
        let normalized = complete_events
            .into_iter()
            .map(|event| normalized_event(codecs, event))
            .collect::<Option<Vec<_>>>()?;
        let mut classifier = StreamClassifier::new(classifier_config);
        let initial_window = normalized
            .iter()
            .filter(|event| classifier.belongs_to_initial_window(event.end_offset))
            .collect::<Vec<_>>();
        let mut stream_parser = select_stream_parser(&initial_window);
        if stream_parser.is_none() {
            stream_parser = normalized
                .iter()
                .filter(|event| !classifier_config.can_sniff_through(event.end_offset))
                .find_map(|event| select_stream_parser(&[event]));
        }
        if stream_parser.is_some() {
            classifier.confirm_llm();
        }
        let mut cache = Self {
            source,
            codec_revision: codecs.revision(),
            framer,
            classifier,
            stream_parser,
            provider_id_override: None,
            provider_response_id: None,
            token_usage: None,
            done: false,
            chunk_count: 0,
        };
        if cache.classifier.is_confirmed_llm() {
            for event in normalized {
                cache.apply_event(event)?;
            }
        }
        let progress = cache.progress();
        Some((cache, progress))
    }

    fn apply_event(&mut self, event: IncrementalNormalizedEvent) -> Option<()> {
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
        if let Some(usage) = event.event.json.as_ref().and_then(extract_token_usage) {
            self.token_usage = Some(usage);
        }
        if parsed.done || parsed.finish_reason.is_some() {
            self.done = true;
        }
        if parsed.content_text.is_some() || parsed.reasoning_text.is_some() {
            self.chunk_count += 1;
        }
        Some(())
    }

    fn observe_delta(
        &mut self,
        body: &[u8],
        codecs: &LlmCodecRegistry,
    ) -> Result<Option<ProviderStreamUpdate>, ()> {
        if self.codec_revision != codecs.revision() {
            return Err(());
        }
        let events = self
            .framer
            .advance(body)
            .map_err(|_| ())?
            .into_iter()
            .map(|event| normalized_event(codecs, event).ok_or(()))
            .collect::<Result<Vec<_>, _>>()?;
        if self.classifier.is_confirmed_llm() {
            for event in events {
                self.apply_event(event).ok_or(())?;
            }
            return Ok(self.progress());
        }
        for event in &events {
            self.classifier.belongs_to_initial_window(event.end_offset);
            if let Some(parser) = select_stream_parser(&[event]) {
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
        self.done = false;
        self.chunk_count = 0;
        let mut replay = IncrementalSseFramer::default();
        for event in replay.advance(body).map_err(|_| ())? {
            let event = normalized_event(codecs, event).ok_or(())?;
            self.apply_event(event).ok_or(())?;
        }
        Ok(())
    }

    fn body(&mut self) -> Option<LlmResponseBody> {
        let mut parsed = self.stream_parser.as_mut()?.finish()?;
        if let Some(usage) = self.token_usage.clone() {
            parsed.token_usage = Some(usage);
        }
        let mut body = response_body(false, parsed, self.provider_response_id.clone());
        if let Some(provider_id) = self.provider_id_override.clone() {
            body.provider_id = provider_id;
        }
        Some(body)
    }

    fn progress(&self) -> Option<ProviderStreamUpdate> {
        self.classifier
            .is_confirmed_llm()
            .then_some(ProviderStreamUpdate {
                done: self.done,
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
    cache: &mut Option<IncrementalSseCache>,
    allow_batch: bool,
) -> CacheAdvance {
    if let Some(existing) = cache.as_ref() {
        if existing.source != source {
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
                    batch_advance(body, codecs)
                } else {
                    CacheAdvance::Unparseable
                }
            }
        },
        None => {
            if let Some((seeded, progress)) =
                IncrementalSseCache::seed(source, body, codecs, classifier_config)
            {
                *cache = Some(seeded);
                progress.map_or(CacheAdvance::Unparseable, CacheAdvance::Incremental)
            } else {
                if allow_batch {
                    batch_advance(body, codecs)
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
) -> Option<Box<dyn LlmProviderResponseStreamParser + Send>> {
    if events.is_empty() {
        return None;
    }
    let provider_events = events
        .iter()
        .map(|event| provider_normalized_sse_event(&event.event))
        .collect::<Vec<_>>();
    new_sse_stream_parser(LlmSseResponseInput {
        // Built-in stream classifiers operate on complete normalized events.
        // Keeping the accumulated text out of this hot path avoids validating
        // or materializing the complete body after every network append.
        text: "",
        events: &provider_events,
    })
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

fn batch_advance(body: &[u8], codecs: &LlmCodecRegistry) -> CacheAdvance {
    match parse_llm_response_body(body, codecs) {
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
    cache: &mut Option<IncrementalSseCache>,
    allow_batch: bool,
) -> Option<ProviderStreamUpdate> {
    match advance_cache(source, body, codecs, classifier_config, cache, allow_batch) {
        CacheAdvance::Incremental(progress) => Some(progress),
        CacheAdvance::Batch(parsed) => Some(ProviderStreamUpdate {
            done: parsed.done,
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
    cache: &mut Option<IncrementalSseCache>,
) -> Option<LlmResponseBody> {
    match advance_cache(source, body, codecs, classifier_config, cache, true) {
        CacheAdvance::Incremental(_) => cache.as_mut()?.body(),
        CacheAdvance::Batch(parsed) => Some(parsed),
        CacheAdvance::Unparseable => None,
    }
}

/// Split SSE text into events like `parse_sse_events`, additionally reporting
/// whether the text ended mid-event (the last block had no trailing blank line).
fn parse_sse_events_with_trailing(text: &str) -> (Vec<SseCodecEvent>, bool) {
    let mut framer = IncrementalSseFramer::default();
    let events = framer
        .advance(text.as_bytes())
        .unwrap_or_default()
        .into_iter()
        .map(sse_codec_event)
        .collect::<Vec<_>>();
    let trailing_partial = framer.safe_consumed() < text.len();
    (events, trailing_partial)
}

struct LlmResponseBodyParser<'a> {
    codecs: &'a LlmCodecRegistry,
}

impl LlmResponseBodyParser<'_> {
    fn parse(&self, body: &[u8]) -> Option<LlmResponseBody> {
        let text = String::from_utf8_lossy(body).into_owned();
        if let Some(sse) = self.parse_sse_response_body(&text) {
            return Some(sse);
        }
        let json = serde_json::from_slice::<Value>(body).ok();
        let value = json.as_ref()?;
        let parsed = parse_json_response(LlmJsonResponseInput {
            text: &text,
            json: value,
        })?;
        Some(response_body(true, parsed, provider_response_id(value)))
    }

    fn parse_sse_response_body(&self, text: &str) -> Option<LlmResponseBody> {
        let raw_events = parse_sse_events(text);
        if raw_events.is_empty() {
            return None;
        }
        if let Some((provider_id, normalized)) = self.normalized_sse_events(&raw_events) {
            return decoded_sse_response(text, raw_events, provider_id, normalized);
        }
        let provider_events = raw_events
            .iter()
            .map(provider_sse_event)
            .collect::<Vec<_>>();
        let parsed = parse_sse_response(LlmSseResponseInput {
            text,
            events: &provider_events,
        })?;
        Some(response_body(
            false,
            parsed.response,
            provider_response_id_from_events(&raw_events),
        ))
    }

    fn normalized_sse_events(
        &self,
        raw_events: &[SseCodecEvent],
    ) -> Option<(Option<String>, Vec<NormalizedSseEvent>)> {
        let mut decoded_any = false;
        let mut provider_id = None;
        let mut normalized = Vec::with_capacity(raw_events.len());
        for event in raw_events {
            if let Some(decoded) = self.codecs.decode_sse_event(event) {
                let data = String::from_utf8(decoded.body).ok()?;
                let trimmed = data.trim();
                provider_id = provider_id.or(decoded.provider_id);
                normalized.push(NormalizedSseEvent {
                    index: event.index,
                    event_type: event.event_type.clone(),
                    id: event.id.clone(),
                    json: serde_json::from_str::<Value>(trimmed).ok(),
                    done_marker: trimmed == SSE_DONE_MARKER,
                    data,
                });
                decoded_any = true;
            } else {
                normalized.push(normalized_event_from_raw(event));
            }
        }
        decoded_any.then_some((provider_id, normalized))
    }
}

fn decoded_sse_response(
    text: &str,
    _raw_events: Vec<SseCodecEvent>,
    provider_id: Option<String>,
    normalized: Vec<NormalizedSseEvent>,
) -> Option<LlmResponseBody> {
    let provider_events = normalized
        .iter()
        .map(provider_normalized_sse_event)
        .collect::<Vec<_>>();
    let parsed = parse_sse_response(LlmSseResponseInput {
        text,
        events: &provider_events,
    })?;
    let response_id = provider_response_id_from_normalized_events(&normalized);
    let mut body = response_body(false, parsed.response, response_id);
    if let Some(provider_id) = provider_id {
        body.provider_id = provider_id;
    }
    Some(body)
}

fn response_body(
    json_valid: bool,
    parsed: LlmParsedResponse,
    provider_response_id: Option<String>,
) -> LlmResponseBody {
    let tool_calls_json = tool_calls_json(&parsed.tool_calls);
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
        done: parsed.done,
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

fn provider_response_id_from_events(events: &[SseCodecEvent]) -> Option<String> {
    events.iter().rev().find_map(|event| {
        event
            .json
            .as_ref()
            .and_then(|value| value.get("response"))
            .and_then(provider_response_id)
    })
}

fn provider_response_id_from_normalized_events(events: &[NormalizedSseEvent]) -> Option<String> {
    events.iter().rev().find_map(|event| {
        event
            .json
            .as_ref()
            .and_then(|value| value.get("response"))
            .and_then(provider_response_id)
    })
}

fn parse_sse_events(text: &str) -> Vec<SseCodecEvent> {
    parse_sse_events_with_trailing(text).0
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

fn provider_sse_event(event: &SseCodecEvent) -> ProviderSseEvent<'_> {
    ProviderSseEvent {
        index: event.index,
        event_type: event.event_type.as_deref(),
        id: event.id.as_deref(),
        data: &event.data,
        json: event.json.as_ref(),
        done_marker: event.done_marker,
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

fn normalized_event_from_raw(event: &SseCodecEvent) -> NormalizedSseEvent {
    NormalizedSseEvent {
        index: event.index,
        event_type: event.event_type.clone(),
        id: event.id.clone(),
        data: event.data.clone(),
        json: event.json.clone(),
        done_marker: event.done_marker,
    }
}
