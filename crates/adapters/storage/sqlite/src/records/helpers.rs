//! Reusable scalar encoders for SQLite record storage.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub(crate) fn escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('=', "\\e")
}

pub(crate) fn unescape(value: &str) -> String {
    let mut output = String::new();
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('n') => output.push('\n'),
                Some('e') => output.push('='),
                Some('\\') => output.push('\\'),
                Some(other) => {
                    output.push('\\');
                    output.push(other);
                }
                None => output.push('\\'),
            }
        } else {
            output.push(ch);
        }
    }
    output
}

pub fn encode_tags(tags: &BTreeSet<String>) -> String {
    tags.iter()
        .map(|tag| escape(tag))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn decode_tags(raw: &str) -> BTreeSet<String> {
    raw.lines().map(unescape).collect()
}

pub fn encode_map(values: &BTreeMap<String, String>) -> String {
    values
        .iter()
        .map(|(key, value)| format!("{}={}", escape(key), escape(value)))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn decode_map(raw: &str) -> BTreeMap<String, String> {
    raw.lines()
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (unescape(key), unescape(value)))
        .collect()
}

pub fn encode_time(value: SystemTime) -> i64 {
    value
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

pub fn decode_time(value: i64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(value as u64)
}

pub fn bool_to_i64(value: bool) -> i64 {
    if value { 1 } else { 0 }
}

pub fn i64_to_bool(value: i64) -> bool {
    value != 0
}

pub(crate) fn encode_bytes(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(hex_digit(byte >> 4));
        output.push(hex_digit(byte & 0x0f));
    }
    output
}

pub(crate) fn decode_bytes(raw: &str) -> Result<Vec<u8>, rusqlite::Error> {
    if raw.len() % 2 != 0 {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let mut bytes = Vec::with_capacity(raw.len() / 2);
    let mut chars = raw.chars();
    while let (Some(high), Some(low)) = (chars.next(), chars.next()) {
        let high = from_hex(high)?;
        let low = from_hex(low)?;
        bytes.push((high << 4) | low);
    }
    Ok(bytes)
}

fn hex_digit(value: u8) -> char {
    match value {
        0..=9 => (b'0' + value) as char,
        10..=15 => (b'a' + (value - 10)) as char,
        _ => '?',
    }
}

fn from_hex(value: char) -> Result<u8, rusqlite::Error> {
    match value {
        '0'..='9' => Ok(value as u8 - b'0'),
        'a'..='f' => Ok(value as u8 - b'a' + 10),
        'A'..='F' => Ok(value as u8 - b'A' + 10),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}
