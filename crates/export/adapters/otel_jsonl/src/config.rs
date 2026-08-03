use std::path::PathBuf;

use export_core::SemanticActionKindSelection;

const FILE_EXPORTER: &str = "file";
const JSON_RPC_HTTP_EXPORTER: &str = "json_rpc_http";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OtelJsonlExporterConfig {
    pub(crate) exporter: ExporterConfig,
    pub(crate) queue_capacity: u32,
    pub(crate) action_kinds: SemanticActionKindSelection,
}

impl OtelJsonlExporterConfig {
    fn parse(raw: &str) -> Result<Self, String> {
        let value = raw
            .parse::<toml::Value>()
            .map_err(|error| format!("parse otel-jsonl plugin config: {error}"))?;
        let table = value
            .as_table()
            .ok_or_else(|| "otel-jsonl plugin config must be a TOML table".to_string())?;
        OtelJsonlConfigParser::new(table).parse()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ExporterConfig {
    File(FileExporterConfig),
    JsonRpcHttp(JsonRpcHttpExporterConfig),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FileExporterConfig {
    pub(crate) path: PathBuf,
    pub(crate) overwrite_enabled: bool,
    pub(crate) flush_every_spans: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct JsonRpcHttpExporterConfig {
    pub(crate) endpoint: String,
    pub(crate) method: String,
    pub(crate) connect_timeout_ms: u32,
    pub(crate) request_timeout_ms: u32,
    pub(crate) response_body_max_bytes: u32,
    pub(crate) max_attempts: u32,
    pub(crate) retry_backoff_ms: u32,
}

struct OtelJsonlConfigParser<'a> {
    root: &'a toml::value::Table,
}

impl<'a> OtelJsonlConfigParser<'a> {
    fn new(root: &'a toml::value::Table) -> Self {
        Self { root }
    }

    fn parse(self) -> Result<OtelJsonlExporterConfig, String> {
        self.reject_unknown(
            self.root,
            "plugin.otel-jsonl",
            &[
                "exporter",
                "queue_capacity",
                "action_kinds",
                FILE_EXPORTER,
                JSON_RPC_HTTP_EXPORTER,
            ],
        )?;
        let exporter_name = self.required_string(self.root, "plugin.otel-jsonl", "exporter")?;
        let file = self.parse_file_exporter()?;
        let json_rpc_http = self.parse_json_rpc_http_exporter()?;
        let exporter = match exporter_name {
            FILE_EXPORTER => ExporterConfig::File(
                file.ok_or_else(|| "missing config key plugin.otel-jsonl.file".to_string())?,
            ),
            JSON_RPC_HTTP_EXPORTER => {
                ExporterConfig::JsonRpcHttp(json_rpc_http.ok_or_else(|| {
                    "missing config key plugin.otel-jsonl.json_rpc_http".to_string()
                })?)
            }
            value => {
                return Err(format!(
                    "invalid plugin.otel-jsonl.exporter {value:?}: expected \
                     {FILE_EXPORTER:?} or {JSON_RPC_HTTP_EXPORTER:?}"
                ));
            }
        };
        Ok(OtelJsonlExporterConfig {
            exporter,
            queue_capacity: self.required_positive_u32(
                self.root,
                "plugin.otel-jsonl",
                "queue_capacity",
            )?,
            action_kinds: self.parse_action_kinds()?,
        })
    }

    fn parse_file_exporter(&self) -> Result<Option<FileExporterConfig>, String> {
        let scope = "plugin.otel-jsonl.file";
        let Some(value) = self.root.get(FILE_EXPORTER) else {
            return Ok(None);
        };
        let table = value
            .as_table()
            .ok_or_else(|| format!("{scope} must be a table"))?;
        self.reject_unknown(
            table,
            scope,
            &["path", "overwrite_enabled", "flush_every_spans"],
        )?;
        let path = PathBuf::from(self.required_non_empty_string(table, scope, "path")?);
        if !path.is_absolute() {
            return Err("invalid plugin.otel-jsonl.file.path: expected absolute path".to_string());
        }
        Ok(Some(FileExporterConfig {
            path,
            overwrite_enabled: self.required_bool(table, scope, "overwrite_enabled")?,
            flush_every_spans: self.required_positive_u32(table, scope, "flush_every_spans")?,
        }))
    }

    fn parse_json_rpc_http_exporter(&self) -> Result<Option<JsonRpcHttpExporterConfig>, String> {
        let scope = "plugin.otel-jsonl.json_rpc_http";
        let Some(value) = self.root.get(JSON_RPC_HTTP_EXPORTER) else {
            return Ok(None);
        };
        let table = value
            .as_table()
            .ok_or_else(|| format!("{scope} must be a table"))?;
        self.reject_unknown(
            table,
            scope,
            &[
                "endpoint",
                "method",
                "connect_timeout_ms",
                "request_timeout_ms",
                "response_body_max_bytes",
                "max_attempts",
                "retry_backoff_ms",
            ],
        )?;
        let endpoint = self
            .required_non_empty_string(table, scope, "endpoint")?
            .to_string();
        self.validate_http_endpoint(&endpoint)?;
        let connect_timeout_ms = self.required_positive_u32(table, scope, "connect_timeout_ms")?;
        let request_timeout_ms = self.required_positive_u32(table, scope, "request_timeout_ms")?;
        if request_timeout_ms < connect_timeout_ms {
            return Err(format!(
                "invalid {scope}.request_timeout_ms: must be greater than or equal to \
                 connect_timeout_ms"
            ));
        }
        Ok(Some(JsonRpcHttpExporterConfig {
            endpoint,
            method: self
                .required_non_empty_string(table, scope, "method")?
                .to_string(),
            connect_timeout_ms,
            request_timeout_ms,
            response_body_max_bytes: self.required_positive_u32(
                table,
                scope,
                "response_body_max_bytes",
            )?,
            max_attempts: self.required_positive_u32(table, scope, "max_attempts")?,
            retry_backoff_ms: self.required_u32(table, scope, "retry_backoff_ms")?,
        }))
    }

    fn parse_action_kinds(&self) -> Result<SemanticActionKindSelection, String> {
        let table = self.required_table(self.root, "plugin.otel-jsonl", "action_kinds")?;
        let mut entries = Vec::with_capacity(table.len());
        for (key, value) in table {
            let enabled = value
                .as_bool()
                .ok_or_else(|| format!("otel-jsonl config action_kinds.{key} must be a bool"))?;
            entries.push((key.clone(), enabled));
        }
        SemanticActionKindSelection::from_config_entries(entries)
            .map_err(|message| format!("invalid plugin.otel-jsonl.action_kinds: {message}"))
    }

    fn validate_http_endpoint(&self, endpoint: &str) -> Result<(), String> {
        let uri = endpoint.parse::<ureq::http::Uri>().map_err(|error| {
            format!("invalid plugin.otel-jsonl.json_rpc_http.endpoint: {error}")
        })?;
        if !matches!(uri.scheme_str(), Some("http" | "https")) {
            return Err(
                "invalid plugin.otel-jsonl.json_rpc_http.endpoint: expected http or https URL"
                    .to_string(),
            );
        }
        if uri.authority().is_none() {
            return Err(
                "invalid plugin.otel-jsonl.json_rpc_http.endpoint: URL must include a host"
                    .to_string(),
            );
        }
        Ok(())
    }

    fn reject_unknown(
        &self,
        table: &toml::value::Table,
        scope: &str,
        allowed: &[&str],
    ) -> Result<(), String> {
        for key in table.keys() {
            if !allowed.contains(&key.as_str()) {
                return Err(format!("unknown config key {scope}.{key}"));
            }
        }
        Ok(())
    }

    fn required_table<'b>(
        &self,
        table: &'b toml::value::Table,
        scope: &str,
        key: &str,
    ) -> Result<&'b toml::value::Table, String> {
        self.required_value(table, scope, key)?
            .as_table()
            .ok_or_else(|| format!("{scope}.{key} must be a table"))
    }

    fn required_non_empty_string<'b>(
        &self,
        table: &'b toml::value::Table,
        scope: &str,
        key: &str,
    ) -> Result<&'b str, String> {
        let value = self.required_string(table, scope, key)?;
        if value.trim().is_empty() {
            return Err(format!("invalid {scope}.{key}: value must not be empty"));
        }
        Ok(value)
    }

    fn required_string<'b>(
        &self,
        table: &'b toml::value::Table,
        scope: &str,
        key: &str,
    ) -> Result<&'b str, String> {
        self.required_value(table, scope, key)?
            .as_str()
            .ok_or_else(|| format!("{scope}.{key} must be a string"))
    }

    fn required_bool(
        &self,
        table: &toml::value::Table,
        scope: &str,
        key: &str,
    ) -> Result<bool, String> {
        self.required_value(table, scope, key)?
            .as_bool()
            .ok_or_else(|| format!("{scope}.{key} must be a bool"))
    }

    fn required_positive_u32(
        &self,
        table: &toml::value::Table,
        scope: &str,
        key: &str,
    ) -> Result<u32, String> {
        let value = self.required_u32(table, scope, key)?;
        if value == u32::default() {
            return Err(format!("invalid {scope}.{key}: value must be positive"));
        }
        Ok(value)
    }

    fn required_u32(
        &self,
        table: &toml::value::Table,
        scope: &str,
        key: &str,
    ) -> Result<u32, String> {
        let value = self
            .required_value(table, scope, key)?
            .as_integer()
            .ok_or_else(|| format!("{scope}.{key} must be an integer"))?;
        u32::try_from(value).map_err(|error| format!("invalid {scope}.{key}: {error}"))
    }

    fn required_value<'b>(
        &self,
        table: &'b toml::value::Table,
        scope: &str,
        key: &str,
    ) -> Result<&'b toml::Value, String> {
        table
            .get(key)
            .ok_or_else(|| format!("missing config key {scope}.{key}"))
    }
}

pub fn parse_otel_jsonl_plugin_config(raw: &str) -> Result<OtelJsonlExporterConfig, String> {
    OtelJsonlExporterConfig::parse(raw)
}
