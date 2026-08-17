//! LLM response body and SSE framing adapter.

use semantic_action::{
    LlmJsonResponseInput, LlmParsedResponse, LlmProviderResponseStreamParser,
    LlmSseEvent as ProviderSseEvent, LlmSseResponseInput, LlmTokenUsage,
};
use serde_json::Value;

use super::codec::{LlmCodecRegistry, NormalizedSseEvent, SseCodecEvent};
use super::provider::{
    extract_token_usage, new_sse_stream_parser, parse_json_response, parse_sse_response,
    tool_calls_json,
};

const SSE_DONE_MARKER: &str = "[DONE]";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct LlmResponseBody {
    pub(super) provider_id: String,
    pub(super) provider_response_id: Option<String>,
    pub(super) json_valid: bool,
    pub(super) model: Option<String>,
    pub(super) content_text: Option<String>,
    pub(super) reasoning_text: Option<String>,
    pub(super) tool_calls_json: Option<String>,
    pub(super) token_usage: Option<LlmTokenUsage>,
    pub(super) chunk_count: usize,
    pub(super) done: bool,
    pub(super) stream: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct LlmResponseProgress {
    pub(super) done: bool,
    pub(super) stream: bool,
    pub(super) chunk_count: usize,
}

pub(super) fn parse_llm_response_body(
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
pub(super) enum SseBodySource {
    SplitHttp,
    RawBytes,
    ChunkedBody,
}

/// Incremental SSE parse state for one in-flight response message.
///
/// Every payload segment re-ran `parse_llm_response_body` over the whole
/// accumulated SSE body, which is O(M²) over a stream of M events. This cache
/// parses only newly appended events through a provider stream parser and
/// accumulates the aggregate response body instead.
pub(crate) struct IncrementalSseCache {
    source: SseBodySource,
    stream_parser: Box<dyn LlmProviderResponseStreamParser + Send>,
    token_usage: Option<LlmTokenUsage>,
    consumed_body_len: usize,
    /// True when the parsed body ended mid-event (no trailing blank line).
    /// If the next delta continues that event, semantics require a batch
    /// re-parse fallback.
    pending_partial: bool,
    done: bool,
    chunk_count: usize,
}

impl IncrementalSseCache {
    fn seed(
        source: SseBodySource,
        body: &[u8],
        codecs: &LlmCodecRegistry,
    ) -> Option<(Self, LlmResponseProgress)> {
        // Codec-decoded streams and non-OpenAI-compatible providers keep the
        // batch parser so incremental behavior stays exactly equivalent where
        // it is active.
        if !codecs.is_empty() {
            return None;
        }
        let text = std::str::from_utf8(body).ok()?;
        let (events, trailing_partial) = parse_sse_events_with_trailing(text);
        let first = events.first()?;
        let (stream_parser, provider_id) = new_sse_stream_parser(LlmSseResponseInput {
            text,
            events: std::slice::from_ref(&provider_sse_event(first)),
        })?;
        if provider_id != "openai-compatible" {
            return None;
        }
        let mut cache = Self {
            source,
            stream_parser,
            token_usage: None,
            consumed_body_len: 0,
            pending_partial: false,
            done: false,
            chunk_count: 0,
        };
        for event in events {
            cache.apply_event(event);
        }
        cache.consumed_body_len = body.len();
        cache.pending_partial = trailing_partial;
        let progress = cache.progress();
        Some((cache, progress))
    }

    fn apply_event(&mut self, event: SseCodecEvent) {
        let parsed = self.stream_parser.observe_event(provider_sse_event(&event));
        if let Some(usage) = event.json.as_ref().and_then(extract_token_usage) {
            self.token_usage = Some(usage);
        }
        if parsed.done || parsed.finish_reason.is_some() {
            self.done = true;
        }
        if parsed.content_text.is_some() || parsed.reasoning_text.is_some() {
            self.chunk_count += 1;
        }
    }

    fn observe_delta(&mut self, body: &[u8]) -> Result<(), ()> {
        if self.consumed_body_len >= body.len() {
            return Ok(());
        }
        let text = std::str::from_utf8(&body[self.consumed_body_len..]).map_err(|_| ())?;
        if text.is_empty() {
            self.consumed_body_len = body.len();
            return Ok(());
        }
        if self.pending_partial && !text.starts_with("\n\n") {
            return Err(());
        }
        let (events, trailing_partial) = parse_sse_events_with_trailing(text);
        for event in events {
            self.apply_event(event);
        }
        self.consumed_body_len = body.len();
        self.pending_partial = trailing_partial;
        Ok(())
    }

    fn body(&mut self) -> Option<LlmResponseBody> {
        let mut parsed = self.stream_parser.finish()?;
        if let Some(usage) = self.token_usage.clone() {
            parsed.token_usage = Some(usage);
        }
        Some(response_body(false, parsed, None))
    }

    fn progress(&self) -> LlmResponseProgress {
        LlmResponseProgress {
            done: self.done,
            stream: true,
            chunk_count: self.chunk_count,
        }
    }
}

fn advance_cache(
    source: SseBodySource,
    body: &[u8],
    codecs: &LlmCodecRegistry,
    cache: &mut Option<IncrementalSseCache>,
) -> CacheAdvance {
    if let Some(existing) = cache.as_ref() {
        if existing.source != source {
            *cache = None;
        }
    }
    match cache {
        Some(existing) => match existing.observe_delta(body) {
            Ok(()) => CacheAdvance::Incremental(existing.progress()),
            Err(()) => {
                *cache = None;
                batch_advance(body, codecs)
            }
        },
        None => {
            if let Some((seeded, progress)) = IncrementalSseCache::seed(source, body, codecs) {
                *cache = Some(seeded);
                CacheAdvance::Incremental(progress)
            } else {
                batch_advance(body, codecs)
            }
        }
    }
}

enum CacheAdvance {
    Incremental(LlmResponseProgress),
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
pub(super) fn parse_llm_response_progress(
    source: SseBodySource,
    body: &[u8],
    codecs: &LlmCodecRegistry,
    cache: &mut Option<IncrementalSseCache>,
) -> Option<LlmResponseProgress> {
    match advance_cache(source, body, codecs, cache) {
        CacheAdvance::Incremental(progress) => Some(progress),
        CacheAdvance::Batch(parsed) => Some(LlmResponseProgress {
            done: parsed.done,
            stream: parsed.stream,
            chunk_count: parsed.chunk_count,
        }),
        CacheAdvance::Unparseable => None,
    }
}

/// Parse an SSE response body incrementally when possible, falling back to the
/// batch parser for codec paths, unsupported providers, or mid-event merges.
pub(super) fn parse_llm_response_body_incremental(
    source: SseBodySource,
    body: &[u8],
    codecs: &LlmCodecRegistry,
    cache: &mut Option<IncrementalSseCache>,
) -> Option<LlmResponseBody> {
    match advance_cache(source, body, codecs, cache) {
        CacheAdvance::Incremental(_) => cache.as_mut()?.body(),
        CacheAdvance::Batch(parsed) => Some(parsed),
        CacheAdvance::Unparseable => None,
    }
}

/// Split SSE text into events like `parse_sse_events`, additionally reporting
/// whether the text ended mid-event (the last block had no trailing blank line).
fn parse_sse_events_with_trailing(text: &str) -> (Vec<SseCodecEvent>, bool) {
    let mut items = Vec::new();
    for block in text.split("\n\n").filter(|block| !block.trim().is_empty()) {
        let mut data_lines = Vec::new();
        let mut event_type = None;
        let mut id = None;
        for line in block.lines() {
            let line = line.trim_end_matches('\r');
            let Some((name, value)) = line.split_once(':') else {
                continue;
            };
            match name.trim().to_ascii_lowercase().as_str() {
                "data" => data_lines.push(value.trim_start()),
                "event" => event_type = Some(value.trim().to_string()),
                "id" => id = Some(value.trim().to_string()),
                _ => {}
            }
        }
        if !data_lines.is_empty() {
            let data = data_lines.join("\n");
            items.push(raw_sse_event(items.len(), event_type, id, data));
        }
    }
    let trailing_partial = !text.is_empty()
        && !text.ends_with("\n\n")
        && text
            .rsplit("\n\n")
            .next()
            .is_some_and(|block| !block.trim().is_empty());
    (items, trailing_partial)
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
    let mut items = Vec::new();
    for block in text.split("\n\n").filter(|block| !block.trim().is_empty()) {
        let mut data_lines = Vec::new();
        let mut event_type = None;
        let mut id = None;
        for line in block.lines() {
            let line = line.trim_end_matches('\r');
            let Some((name, value)) = line.split_once(':') else {
                continue;
            };
            match name.trim().to_ascii_lowercase().as_str() {
                "data" => data_lines.push(value.trim_start()),
                "event" => event_type = Some(value.trim().to_string()),
                "id" => id = Some(value.trim().to_string()),
                _ => {}
            }
        }
        if !data_lines.is_empty() {
            let data = data_lines.join("\n");
            items.push(raw_sse_event(items.len(), event_type, id, data));
        }
    }
    items
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
