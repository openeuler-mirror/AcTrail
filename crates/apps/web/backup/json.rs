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

pub fn optional_string(value: Option<&str>) -> String {
    value.map(string).unwrap_or_else(|| "null".to_string())
}

pub fn optional_number<T: std::fmt::Display>(value: Option<T>) -> String {
    value.map(number).unwrap_or_else(|| "null".to_string())
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
        Ok(duration) => number(duration.as_secs()),
        Err(_) => string("before-unix-epoch"),
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
