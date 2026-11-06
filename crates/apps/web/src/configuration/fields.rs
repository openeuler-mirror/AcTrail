use serde_json::{Value as JsonValue, json};
use toml_edit::{Array, Item, Value};

use FieldType::{Boolean, Integer, StringArray, Text};

pub(super) const EDITABLE_FIELDS: [ConfigField; 40] = [
    ConfigField::new("storage.retention.enabled", Boolean),
    ConfigField::new("storage.retention.max_trace_age", Text),
    ConfigField::new("storage.retention.protected_tags", StringArray),
    ConfigField::new("payload.tls.redaction_policy", Text),
    ConfigField::new("payload.stdio.redaction_policy", Text),
    ConfigField::new("payload.socket.redaction_policy", Text),
    ConfigField::new("payload.tls.enabled", Boolean),
    ConfigField::new("payload.stdio.enabled", Boolean),
    ConfigField::new("payload.socket.enabled", Boolean),
    ConfigField::new("payload.mcp.enabled", Boolean),
    ConfigField::new("payload.stdio.capture_stdin", Boolean),
    ConfigField::new("payload.stdio.capture_stdout", Boolean),
    ConfigField::new("payload.stdio.capture_stderr", Boolean),
    ConfigField::new("payload.stdio.stdin_storage_mode", Text),
    ConfigField::new("payload.stdio.stdout_storage_mode", Text),
    ConfigField::new("payload.stdio.stderr_storage_mode", Text),
    ConfigField::new("payload.tls.retention_max_bytes_per_trace", Integer),
    ConfigField::new("payload.stdio.retention_max_bytes_per_trace", Integer),
    ConfigField::new("payload.socket.retention_max_bytes_per_trace", Integer),
    ConfigField::new("resource_profiling.enabled", Boolean),
    ConfigField::new("resource_profiling.collection.host.interval_ms", Integer),
    ConfigField::new(
        "resource_profiling.collection.workload.interval_ms",
        Integer,
    ),
    ConfigField::new("resource_profiling.alerts.cpu_alert_percent_millis", Text),
    ConfigField::new("resource_profiling.alerts.memory_alert_current_bytes", Text),
    ConfigField::new("resource_profiling.alerts.memory_alert_rss_kb", Text),
    ConfigField::new(
        "resource_profiling.storage.database.sqlite.retention.enabled",
        Boolean,
    ),
    ConfigField::new(
        "resource_profiling.storage.database.sqlite.retention.max_age_seconds",
        Integer,
    ),
    ConfigField::new("idle_detection.enabled", Boolean),
    ConfigField::new("idle_detection.threshold_secs", Text),
    ConfigField::new("export.snapshot.payload_bytes_enabled", Boolean),
    ConfigField::new("export.snapshot.payload_text_enabled", Boolean),
    ConfigField::new("control.diagnostic_log_level", Text),
    ConfigField::new("control.socket_path", Text),
    ConfigField::new("control.pid_file", Text),
    ConfigField::new("control.log_path", Text),
    ConfigField::new("payload.tls.sync_event_socket_path", Text),
    ConfigField::new("plugins.discovery.directory", Text),
    ConfigField::new("web.listen_addr", Text),
    ConfigField::new("storage.sqlite.path", Text),
    ConfigField::new("storage.sqlite.busy_timeout_ms", Integer),
];

enum FieldType {
    Text,
    Integer,
    Boolean,
    StringArray,
}

pub(super) struct ConfigField {
    pub(super) key: &'static str,
    kind: FieldType,
}

impl ConfigField {
    const fn new(key: &'static str, kind: FieldType) -> Self {
        Self { key, kind }
    }

    pub(super) fn decode(&self, value: &JsonValue) -> Result<Value, String> {
        let invalid = |expected| format!("invalid {}: expected {expected}", self.key);
        match self.kind {
            Text => value
                .as_str()
                .map(Value::from)
                .ok_or_else(|| invalid("a string")),
            Boolean => value
                .as_bool()
                .map(Value::from)
                .ok_or_else(|| invalid("a boolean")),
            Integer => {
                let integer = value.as_i64().ok_or_else(|| invalid("a TOML integer"))?;
                if integer < 0 {
                    return Err(invalid("a nonnegative TOML integer"));
                }
                // The inactive SQLite backend is omitted by domain parsing.
                if self.key == "storage.sqlite.busy_timeout_ms" && integer <= 0 {
                    return Err(invalid("a positive TOML integer"));
                }
                Ok(Value::from(integer))
            }
            StringArray => {
                let values = value
                    .as_array()
                    .ok_or_else(|| invalid("an array of strings"))?;
                let mut array = Array::new();
                for value in values {
                    array.push(
                        value
                            .as_str()
                            .ok_or_else(|| invalid("an array of strings"))?,
                    );
                }
                Ok(Value::Array(array))
            }
        }
    }

    pub(super) fn encode(&self, item: &Item) -> Result<JsonValue, String> {
        let value = match self.kind {
            Text => item.as_str().map(|value| json!(value)),
            // Decimal text keeps every TOML i64 exact in the browser. POST
            // still requires a JSON integer and preserves the file's type.
            Integer => item.as_integer().map(|value| json!(value.to_string())),
            Boolean => item.as_bool().map(|value| json!(value)),
            StringArray => item.as_array().and_then(|array| {
                array
                    .iter()
                    .map(Value::as_str)
                    .collect::<Option<Vec<_>>>()
                    .map(|values| json!(values))
            }),
        };
        value.ok_or_else(|| format!("invalid editable config field {}", self.key))
    }

    pub(super) fn encode_resource(
        &self,
        config: &resource_profiling::ResourceProfilingConfig,
    ) -> Option<JsonValue> {
        let threshold = |value: Option<u64>| {
            json!(
                value
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "disabled".into())
            )
        };
        let retention = || {
            config
                .storage
                .database
                .as_ref()
                .map(|database| database.sqlite.retention.clone())
                .unwrap_or_default()
        };
        Some(match self.key {
            "resource_profiling.enabled" => json!(config.enabled),
            "resource_profiling.collection.host.interval_ms" => {
                json!(config.collection.host.interval_ms.to_string())
            }
            "resource_profiling.collection.workload.interval_ms" => {
                json!(config.collection.workload.interval_ms.to_string())
            }
            "resource_profiling.alerts.cpu_alert_percent_millis" => {
                threshold(config.alerts.cpu_alert_percent_millis)
            }
            "resource_profiling.alerts.memory_alert_current_bytes" => {
                threshold(config.alerts.memory_alert_current_bytes)
            }
            "resource_profiling.alerts.memory_alert_rss_kb" => {
                threshold(config.alerts.memory_alert_rss_kb)
            }
            "resource_profiling.storage.database.sqlite.retention.enabled" => {
                json!(retention().enabled)
            }
            "resource_profiling.storage.database.sqlite.retention.max_age_seconds" => {
                json!(retention().max_age_seconds.to_string())
            }
            _ => return None,
        })
    }
}
