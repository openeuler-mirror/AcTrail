//! Minimal JSON writer for web API responses.

use std::collections::BTreeMap;
use std::fmt::Write;
use std::time::{SystemTime, UNIX_EPOCH};

use model_core::process::ProcessIdentity;

pub fn string(value: &str) -> String {
    format!("\"{}\"", escape(value))
}

pub fn number(value: impl std::fmt::Display) -> String {
    value.to_string()
}

pub fn boolean(value: bool) -> String {
    value.to_string()
}

pub fn optional_string(value: Option<&str>) -> String {
    value.map(string).unwrap_or_else(|| "null".to_string())
}

pub fn optional_number<T: std::fmt::Display>(value: Option<T>) -> String {
    value.map(number).unwrap_or_else(|| "null".to_string())
}

pub fn optional_time(value: Option<SystemTime>) -> String {
    value.map(time).unwrap_or_else(|| "null".to_string())
}

pub fn optional_time_nanos(value: Option<SystemTime>) -> String {
    value.map(time_nanos).unwrap_or_else(|| "null".to_string())
}

pub fn map(values: &BTreeMap<String, String>) -> String {
    let mut output = String::from("{");
    for (index, (key, value)) in values.iter().enumerate() {
        if index > 0 {
            output.push(',');
        }
        field(&mut output, key, &string(value));
    }
    output.push('}');
    output
}

pub fn string_array(values: impl IntoIterator<Item = String>) -> String {
    let values = values
        .into_iter()
        .map(|value| string(&value))
        .collect::<Vec<_>>();
    format!("[{}]", values.join(","))
}

pub fn process(identity: &ProcessIdentity) -> String {
    let mut output = String::from("{");
    field(&mut output, "pid", &number(identity.pid));
    output.push(',');
    field(&mut output, "task_id", &optional_number(identity.task_id));
    output.push(',');
    field(
        &mut output,
        "start_time_ticks",
        &number(identity.start_time_ticks),
    );
    output.push(',');
    field(&mut output, "generation", &number(identity.generation));
    output.push(',');
    field(
        &mut output,
        "pid_namespace",
        &optional_string(identity.pid_namespace.as_ref().map(|value| value.as_str())),
    );
    output.push('}');
    output
}

pub fn time(value: SystemTime) -> String {
    match value.duration_since(UNIX_EPOCH) {
        Ok(duration) => number(duration.as_millis()),
        Err(_) => string("before-unix-epoch"),
    }
}

pub fn time_nanos(value: SystemTime) -> String {
    match value.duration_since(UNIX_EPOCH) {
        Ok(duration) => string(&duration.as_nanos().to_string()),
        Err(_) => string("before-unix-epoch"),
    }
}

/// Formats a duration in microseconds to a human-readable string like
/// "1min3s897ms", or "300µs" when the duration is under one millisecond.
pub fn duration_micros(micros: u64) -> String {
    let millis = micros / 1_000;
    if millis == 0 {
        return format!("{}µs", micros);
    }

    let minutes = millis / 60_000;
    let seconds = (millis % 60_000) / 1_000;
    let ms = millis % 1_000;

    if minutes > 0 {
        if ms > 0 {
            format!("{}min{}s{}ms", minutes, seconds, ms)
        } else if seconds > 0 {
            format!("{}min{}s", minutes, seconds)
        } else {
            format!("{}min", minutes)
        }
    } else if seconds > 0 {
        if ms > 0 {
            format!("{}s{}ms", seconds, ms)
        } else {
            format!("{}s", seconds)
        }
    } else {
        format!("{}ms", ms)
    }
}

pub fn field(output: &mut String, key: &str, value: &str) {
    let _ = write!(output, "\"{}\":{}", escape(key), value);
}

fn escape(value: &str) -> String {
    let mut output = String::new();
    for character in value.chars() {
        match character {
            '\\' => output.push_str("\\\\"),
            '"' => output.push_str("\\\""),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            control if control.is_control() => {
                let _ = write!(output, "\\u{:04x}", control as u32);
            }
            other => output.push(other),
        }
    }
    output
}
