use std::path::PathBuf;

use export_core::SemanticActionKindSelection;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OtelJsonlExporterConfig {
    pub path: PathBuf,
    pub overwrite_enabled: bool,
    pub queue_capacity: u32,
    pub flush_every_spans: u32,
    pub action_kinds: SemanticActionKindSelection,
}

impl OtelJsonlExporterConfig {
    pub fn validate(&self) -> Result<(), String> {
        if !self.path.is_absolute() {
            return Err("invalid otel-jsonl export path: expected absolute path".to_string());
        }
        Ok(())
    }
}

pub fn parse_otel_jsonl_plugin_config(raw: &str) -> Result<OtelJsonlExporterConfig, String> {
    let value = raw
        .parse::<toml::Value>()
        .map_err(|error| format!("parse otel-jsonl plugin config: {error}"))?;
    let table = value
        .as_table()
        .ok_or_else(|| "otel-jsonl plugin config must be a TOML table".to_string())?;

    for key in table.keys() {
        match key.as_str() {
            "path" | "overwrite_enabled" | "queue_capacity" | "flush_every_spans"
            | "action_kinds" => {}
            _ => return Err(format!("unknown config key plugin.otel-jsonl.{key}")),
        }
    }

    let action_kind_table = required_value(table, "action_kinds")?
        .as_table()
        .ok_or_else(|| "otel-jsonl config action_kinds must be a table".to_string())?;
    let mut action_kind_entries = Vec::with_capacity(action_kind_table.len());
    for (key, value) in action_kind_table {
        let enabled = value
            .as_bool()
            .ok_or_else(|| format!("otel-jsonl config action_kinds.{key} must be a bool"))?;
        action_kind_entries.push((key.clone(), enabled));
    }

    let config = OtelJsonlExporterConfig {
        path: PathBuf::from(
            required_value(table, "path")?
                .as_str()
                .ok_or_else(|| "otel-jsonl config path must be a string".to_string())?,
        ),
        overwrite_enabled: required_value(table, "overwrite_enabled")?
            .as_bool()
            .ok_or_else(|| "otel-jsonl config overwrite_enabled must be a bool".to_string())?,
        queue_capacity: required_positive_u32(table, "queue_capacity")?,
        flush_every_spans: required_positive_u32(table, "flush_every_spans")?,
        action_kinds: SemanticActionKindSelection::from_config_entries(action_kind_entries)
            .map_err(|message| format!("invalid plugin.otel-jsonl.action_kinds: {message}"))?,
    };
    config.validate()?;
    Ok(config)
}

fn required_value<'a>(
    table: &'a toml::value::Table,
    key: &'static str,
) -> Result<&'a toml::Value, String> {
    table
        .get(key)
        .ok_or_else(|| format!("missing config key plugin.otel-jsonl.{key}"))
}

fn required_positive_u32(table: &toml::value::Table, key: &'static str) -> Result<u32, String> {
    let value = required_value(table, key)?
        .as_integer()
        .ok_or_else(|| format!("otel-jsonl config {key} must be an integer"))?;
    let value = u32::try_from(value)
        .map_err(|error| format!("invalid plugin.otel-jsonl.{key}: {error}"))?;
    if value == 0 {
        return Err(format!(
            "invalid plugin.otel-jsonl.{key}: value must be positive"
        ));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::parse_otel_jsonl_plugin_config;

    #[test]
    fn unknown_otel_jsonl_key_is_rejected() {
        let error = parse_otel_jsonl_plugin_config(
            r#"
path = "/tmp/actrail-live-spans.otlp.jsonl"
overwrite_enabled = true
queue_capacity = 1024
flush_every_spans = 1
unexpected = true

[action_kinds]
default = false
"llm.request" = true
"#,
        )
        .expect_err("unknown OTEL JSONL config key should fail");

        assert!(error.contains("unknown config key plugin.otel-jsonl.unexpected"));
    }
}
