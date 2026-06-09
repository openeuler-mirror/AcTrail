use std::str::FromStr;

use tls_payload_core::{PayloadDirection, RewriteRule};

use super::state::HookPoint;

pub(super) fn parse_points(value: &str) -> Result<Vec<HookPoint>, String> {
    let mut points = Vec::new();
    for item in value.split(';').filter(|item| !item.is_empty()) {
        let mut parts = item.split(':');
        let symbol = parts
            .next()
            .ok_or_else(|| format!("invalid hook point: {item}"))?;
        let direction = parts
            .next()
            .ok_or_else(|| format!("invalid hook point: {item}"))
            .and_then(|value| {
                PayloadDirection::from_str(value).map_err(|error| error.to_string())
            })?;
        let file_offset = parts
            .next()
            .ok_or_else(|| format!("invalid hook point: {item}"))?
            .parse::<u64>()
            .map_err(|error| format!("invalid hook point offset {item}: {error}"))?;
        if parts.next().is_some() {
            return Err(format!("invalid hook point: {item}"));
        }
        points.push(HookPoint {
            symbol: symbol.to_string(),
            direction,
            file_offset,
        });
    }
    if points.is_empty() {
        return Err("runtime hook point list is empty".to_string());
    }
    Ok(points)
}

pub(super) fn parse_rules(value: &str) -> Result<Vec<RewriteRule>, String> {
    let mut rules = Vec::new();
    for item in value.split(';').filter(|item| !item.is_empty()) {
        let (direction, rest) = item
            .split_once(':')
            .ok_or_else(|| format!("invalid runtime rule: {item}"))?;
        let (from, to) = rest
            .split_once('=')
            .ok_or_else(|| format!("invalid runtime rule: {item}"))?;
        let direction = PayloadDirection::from_str(direction).map_err(|error| error.to_string())?;
        rules.push(
            RewriteRule::new(direction, decode_hex(from)?, decode_hex(to)?, item)
                .map_err(|error| error.to_string())?,
        );
    }
    Ok(rules)
}

fn decode_hex(value: &str) -> Result<Vec<u8>, String> {
    if value.len() % 2 != 0 {
        return Err(format!("hex value must have even length: {value}"));
    }
    let mut bytes = Vec::with_capacity(value.len() / 2);
    for chunk in value.as_bytes().chunks_exact(2) {
        let text = std::str::from_utf8(chunk).map_err(|error| format!("hex utf8: {error}"))?;
        bytes.push(u8::from_str_radix(text, 16).map_err(|error| format!("hex {text}: {error}"))?);
    }
    Ok(bytes)
}
