//! Small JSON helpers for OTLP export.

pub fn quoted(value: &str) -> String {
    format!("\"{}\"", escape(value))
}

pub fn escape(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            value if value.is_control() => output.push_str(&format!("\\u{:04x}", value as u32)),
            value => output.push(value),
        }
    }
    output
}

pub fn string_attr(key: &str, value: &str) -> String {
    format!(
        "{{\"key\":{},\"value\":{{\"stringValue\":{}}}}}",
        quoted(key),
        quoted(value)
    )
}

pub fn int_attr(key: &str, value: u64) -> String {
    format!(
        "{{\"key\":{},\"value\":{{\"intValue\":\"{}\"}}}}",
        quoted(key),
        value
    )
}
