//! Streaming HTTP/1.x body helpers shared by chunked and SSE paths.

use std::collections::{BTreeMap, VecDeque};

use config_core::daemon::{ApplicationProtocolConfig, SseDataPolicy};
use model_core::event::ApplicationPayload;

pub(super) fn sse_event_payloads(
    body: &str,
    config: &ApplicationProtocolConfig,
) -> Result<Vec<ApplicationPayload>, String> {
    let mut output = Vec::new();
    for block in sse_blocks(body) {
        if let Some(payload) = ParsedSseEvent::parse(&block).into_payload(config)? {
            output.push(payload);
        }
    }
    Ok(output)
}

struct ParsedSseEvent {
    event_type: Option<String>,
    data: Option<String>,
    saw_field: bool,
}

impl ParsedSseEvent {
    fn parse(block: &str) -> Self {
        let mut event = Self {
            event_type: None,
            data: None,
            saw_field: false,
        };
        for line in block.lines() {
            let line = line.trim_end_matches('\r');
            let Some((name, value)) = line.split_once(':') else {
                continue;
            };
            event.saw_field = true;
            let value = value.trim_start();
            if name.trim().eq_ignore_ascii_case("data") {
                Self::push_field(&mut event.data, value);
            } else if name.trim().eq_ignore_ascii_case("event") {
                Self::push_field(&mut event.event_type, value);
            }
        }
        event
    }

    fn push_field(field: &mut Option<String>, value: &str) {
        if let Some(current) = field {
            current.push('\n');
            current.push_str(value);
        } else {
            *field = Some(value.to_string());
        }
    }

    fn into_payload(
        self,
        config: &ApplicationProtocolConfig,
    ) -> Result<Option<ApplicationPayload>, String> {
        if !self.saw_field {
            return Ok(None);
        }
        let event_name = self.event_type.unwrap_or_else(|| "message".to_string());
        let mut metadata = BTreeMap::from([("event".to_string(), event_name.clone())]);
        if let Some(data) = self.data {
            metadata.insert("data_size".to_string(), data.len().to_string());
            if matches!(config.sse_data_policy, SseDataPolicy::Preview) {
                let (preview, truncated) = preview_data(&data, config.sse_max_data_bytes)?;
                metadata.insert("data_preview".to_string(), preview);
                metadata.insert("data_truncated".to_string(), truncated.to_string());
            }
        }
        Ok(Some(ApplicationPayload {
            protocol: "sse".to_string(),
            operation: "event".to_string(),
            summary: event_name,
            body: None,
            metadata,
        }))
    }
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
