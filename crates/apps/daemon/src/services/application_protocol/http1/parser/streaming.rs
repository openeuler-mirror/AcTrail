//! Streaming HTTP/1.x body helpers shared by chunked and SSE paths.

use std::collections::{BTreeMap, VecDeque};

use config_core::daemon::{ApplicationProtocolConfig, SseDataPolicy};
use model_core::event::ApplicationPayload;

pub(super) struct ChunkDrain {
    pub(super) bodies: Vec<String>,
    pub(super) done: bool,
}

pub(super) fn drain_complete_chunks(text: &mut String) -> Result<ChunkDrain, String> {
    let mut bodies = Vec::new();
    let mut cursor = 0;
    let mut consumed = 0;
    let mut done = false;

    loop {
        let Some(line_end_offset) = text[cursor..].find("\r\n") else {
            break;
        };
        let line_end = cursor + line_end_offset;
        let size_line = &text[cursor..line_end];
        let size_text = size_line.split(';').next().unwrap_or(size_line).trim();
        let size = usize::from_str_radix(size_text, 16)
            .map_err(|error| format!("invalid HTTP chunk size: {error}"))?;
        let data_start = line_end + "\r\n".len();
        let data_end = data_start
            .checked_add(size)
            .ok_or_else(|| "HTTP chunk size overflow".to_string())?;
        let chunk_end = data_end
            .checked_add("\r\n".len())
            .ok_or_else(|| "HTTP chunk terminator overflow".to_string())?;
        if text.len() < chunk_end {
            break;
        }
        if text.get(data_end..chunk_end) != Some("\r\n") {
            return Err("HTTP chunk missing CRLF terminator".to_string());
        }
        if size == 0 {
            consumed = chunk_end;
            done = true;
            break;
        }
        bodies.push(
            text.get(data_start..data_end)
                .ok_or_else(|| "HTTP chunk data is not valid UTF-8".to_string())?
                .to_string(),
        );
        cursor = chunk_end;
        consumed = chunk_end;
    }

    if consumed > 0 {
        text.drain(..consumed);
    }

    Ok(ChunkDrain { bodies, done })
}

pub(super) fn take_complete_sse_events(
    text: &mut String,
    config: &ApplicationProtocolConfig,
) -> Result<Vec<ApplicationPayload>, String> {
    let Some(prefix_len) = complete_sse_prefix_len(text) else {
        return Ok(Vec::new());
    };
    let body = text[..prefix_len].to_string();
    text.drain(..prefix_len);
    sse_event_payloads(&body, config)
}

pub(super) fn sse_event_payloads(
    body: &str,
    config: &ApplicationProtocolConfig,
) -> Result<Vec<ApplicationPayload>, String> {
    let mut output = Vec::new();
    for block in sse_blocks(body) {
        let fields = parse_sse_block(&block);
        if fields.is_empty() {
            continue;
        }
        let event_name = fields
            .get("event")
            .cloned()
            .unwrap_or_else(|| "message".to_string());
        let mut metadata = BTreeMap::from([("event".to_string(), event_name.clone())]);
        if let Some(data) = fields.get("data") {
            metadata.insert("data_size".to_string(), data.len().to_string());
            if matches!(config.sse_data_policy, SseDataPolicy::Preview) {
                let (preview, truncated) = preview_data(data, config.sse_max_data_bytes)?;
                metadata.insert("data_preview".to_string(), preview);
                metadata.insert("data_truncated".to_string(), truncated.to_string());
            }
        }
        output.push(ApplicationPayload {
            protocol: "sse".to_string(),
            operation: "event".to_string(),
            summary: event_name,
            metadata,
        });
    }
    Ok(output)
}

fn complete_sse_prefix_len(text: &str) -> Option<usize> {
    [text.rfind("\n\n").map(|index| index + "\n\n".len()), {
        text.rfind("\r\n\r\n").map(|index| index + "\r\n\r\n".len())
    }]
    .into_iter()
    .flatten()
    .max()
}

fn sse_blocks(body: &str) -> VecDeque<String> {
    let mut blocks = VecDeque::new();
    let normalized = body.replace("\r\n", "\n");
    for block in normalized.split("\n\n") {
        let normalized = block.trim_matches(['\r', '\n']);
        if !normalized.is_empty() {
            blocks.push_back(normalized.to_string());
        }
    }
    blocks
}

fn parse_sse_block(block: &str) -> BTreeMap<String, String> {
    let mut fields = BTreeMap::<String, String>::new();
    for line in block.lines() {
        let line = line.trim_end_matches('\r');
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let key = name.trim().to_ascii_lowercase();
        let value = value.trim_start();
        fields
            .entry(key)
            .and_modify(|existing| {
                existing.push('\n');
                existing.push_str(value);
            })
            .or_insert_with(|| value.to_string());
    }
    fields
}

fn preview_data(data: &str, max_bytes: u64) -> Result<(String, bool), String> {
    let max_bytes = usize::try_from(max_bytes).map_err(|error| error.to_string())?;
    if data.len() <= max_bytes {
        return Ok((data.to_string(), false));
    }
    let mut end = max_bytes;
    while !data.is_char_boundary(end) {
        end = end
            .checked_sub(1)
            .ok_or_else(|| "SSE preview boundary underflow".to_string())?;
    }
    Ok((data[..end].to_string(), true))
}
