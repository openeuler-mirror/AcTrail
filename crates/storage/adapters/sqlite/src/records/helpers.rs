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
    let nanos = value
        .duration_since(UNIX_EPOCH)
        .expect("sqlite timestamp is before unix epoch")
        .as_nanos();
    i64::try_from(nanos).expect("sqlite timestamp nanoseconds exceed i64")
}

pub fn decode_time(value: i64) -> SystemTime {
    let nanos = u64::try_from(value).expect("sqlite timestamp nanoseconds are negative");
    UNIX_EPOCH + Duration::from_nanos(nanos)
}

pub fn bool_to_i64(value: bool) -> i64 {
    if value { 1 } else { 0 }
}

pub fn i64_to_bool(value: i64) -> bool {
    value != 0
}
