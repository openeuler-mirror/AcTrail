use std::collections::BTreeMap;
use std::time::Duration;

use export_core::SemanticActionKindSelection;

pub const OTEL_HTTP_ROUTE_KIND: &str = "otel-http";

/// Realtime OTLP/HTTP exporter configuration.
///
/// Speaks `http://` (cluster-internal collector) or `https://` with optional
/// mutual TLS (system OpenSSL). Delivery stays best-effort at the queue, but
/// each batch gets bounded retries. A partial batch is flushed when
/// `batch_timeout_ms` elapses even if input becomes idle. Shutdown gets a flush
/// deadline so any deployment can bound how long its caller waits to drain.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OtelHttpExporterConfig {
    /// Full collector URL, e.g. `https://10.0.0.5:4318/v1/traces`.
    pub endpoint: String,
    /// Explicit acknowledgement that a plaintext HTTP endpoint is acceptable.
    pub allow_insecure: bool,
    pub queue_capacity: u32,
    /// Spans buffered into one POST body before sending.
    pub batch_max_spans: u32,
    /// Maximum partial-batch age before the delivery worker flushes it.
    pub batch_timeout_ms: u32,
    pub connect_timeout_ms: u32,
    pub request_timeout_ms: u32,
    /// Attempts per batch (>=1). Exhausted => batch dropped, route stays up.
    pub retry_max_attempts: u32,
    pub retry_backoff_ms: u32,
    /// Upper bound for the final flush when the route shuts down.
    pub shutdown_flush_deadline_ms: u32,
    /// TLS material; only consulted for `https://` endpoints.
    pub tls: OtelHttpTlsConfig,
    /// Wire encoding for the exported batch (JSON or protobuf).
    pub encoding: OtelEncoding,
    /// Request-body compression. `gzip` is supported by every OTLP server.
    pub compression: OtelCompression,
    /// Extra headers sent with every OTLP POST, in the configured order.
    /// Optional and empty by default; collectors that authenticate the sender
    /// need one here (Agent Insight attributes a trace by its `x-witty-api-key`
    /// header). Modelled as ordered `name`/`value` pairs rather than a map so
    /// the Web config form can render it like every other plugin setting.
    /// Headers the transport derives itself are rejected — see
    /// [`RESERVED_HEADERS`].
    pub headers: Vec<(String, String)>,
    /// Explicit action-kind policy applied before encoding and queue admission.
    pub action_kinds: SemanticActionKindSelection,
    /// Controls whether domain-specific action attributes may leave the daemon.
    pub attribute_mode: OtelAttributeMode,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum OtelAttributeMode {
    /// Export only the codec's structural span fields. Action attributes stay local.
    #[default]
    MetadataOnly,
    /// Export every attribute already present on the semantic action.
    ///
    /// This does not enable optional content production. In particular, LLM
    /// request bodies and tool result bodies require separate daemon export
    /// settings before an attribute exists for this mode to send.
    Full,
}

impl std::str::FromStr for OtelAttributeMode {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "metadata-only" => Ok(Self::MetadataOnly),
            "full" => Ok(Self::Full),
            other => Err(format!(
                "invalid otel-http attribute_mode {other:?}: expected \"metadata-only\" or \"full\""
            )),
        }
    }
}

/// OTLP/HTTP payload encoding. JSON stays the default for drop-in compatibility;
/// protobuf is the recommended production format (smaller, typed, widely
/// accepted by collectors).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum OtelEncoding {
    #[default]
    Json,
    Protobuf,
}

impl OtelEncoding {
    pub const fn content_type(self) -> &'static str {
        match self {
            Self::Json => "application/json",
            Self::Protobuf => "application/x-protobuf",
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Protobuf => "protobuf",
        }
    }
}

impl std::str::FromStr for OtelEncoding {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "json" => Ok(Self::Json),
            "protobuf" => Ok(Self::Protobuf),
            other => Err(format!(
                "invalid otel-http encoding {other:?}: expected \"json\" or \"protobuf\""
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum OtelCompression {
    #[default]
    None,
    Gzip,
}

impl OtelCompression {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Gzip => "gzip",
        }
    }

    pub const fn content_encoding(self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::Gzip => Some("gzip"),
        }
    }
}

impl std::str::FromStr for OtelCompression {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "none" => Ok(Self::None),
            "gzip" => Ok(Self::Gzip),
            other => Err(format!(
                "invalid otel-http compression {other:?}: expected \"none\" or \"gzip\""
            )),
        }
    }
}

/// TLS / mutual-TLS material for an `https://` collector endpoint.
///
/// - `ca_cert_path`: PEM bundle used to verify the collector's server
///   certificate. Empty => fall back to the system trust store.
/// - `client_cert_path` / `client_key_path`: PEM client certificate and its
///   private key, presented to the collector for mutual TLS. Both or neither —
///   a half-configured client identity is a hard error.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OtelHttpTlsConfig {
    pub ca_cert_path: Option<String>,
    pub client_cert_path: Option<String>,
    pub client_key_path: Option<String>,
}

impl OtelHttpTlsConfig {
    /// A client certificate + key pair is configured for mutual TLS.
    pub fn has_client_identity(&self) -> bool {
        self.client_cert_path.is_some() && self.client_key_path.is_some()
    }

    fn any_set(&self) -> bool {
        self.ca_cert_path.is_some()
            || self.client_cert_path.is_some()
            || self.client_key_path.is_some()
    }
}

impl OtelHttpExporterConfig {
    pub fn parse_section(
        section_name: &str,
        entries: Vec<(String, String)>,
    ) -> Result<Self, String> {
        let values = OtelHttpConfigValues::from_entries(section_name, entries)?;
        Ok(Self {
            endpoint: values.required("endpoint")?,
            allow_insecure: values
                .optional("allow_insecure")
                .map(|value| {
                    value
                        .parse::<bool>()
                        .map_err(|error| format!("invalid {section_name}.allow_insecure: {error}"))
                })
                .transpose()?
                .unwrap_or(false),
            queue_capacity: values.required_positive_u32("queue_capacity")?,
            batch_max_spans: values.required_positive_u32("batch_max_spans")?,
            batch_timeout_ms: values.required_positive_u32("batch_timeout_ms")?,
            connect_timeout_ms: values.required_positive_u32("connect_timeout_ms")?,
            request_timeout_ms: values.required_positive_u32("request_timeout_ms")?,
            retry_max_attempts: values.required_positive_u32("retry_max_attempts")?,
            retry_backoff_ms: values.required_positive_u32("retry_backoff_ms")?,
            shutdown_flush_deadline_ms: values
                .required_positive_u32("shutdown_flush_deadline_ms")?,
            tls: OtelHttpTlsConfig {
                ca_cert_path: values.optional("tls_ca_cert_path"),
                client_cert_path: values.optional("tls_client_cert_path"),
                client_key_path: values.optional("tls_client_key_path"),
            },
            encoding: match values.optional("encoding") {
                Some(value) => value.parse::<OtelEncoding>()?,
                None => OtelEncoding::default(),
            },
            compression: match values.optional("compression") {
                Some(value) => value.parse::<OtelCompression>()?,
                None => OtelCompression::default(),
            },
            // Populated from the `[[headers]]` array by the plugin config
            // parser; a flat key/value section carries no headers.
            headers: Vec::new(),
            action_kinds: SemanticActionKindSelection::from_config_entries([(
                "default".to_string(),
                false,
            )])?,
            attribute_mode: match values.optional("attribute_mode") {
                Some(value) => value.parse::<OtelAttributeMode>()?,
                None => OtelAttributeMode::default(),
            },
        })
    }

    pub fn validate_enabled_route(&self) -> Result<(), String> {
        let endpoint = Endpoint::parse(&self.endpoint)?;
        if self.action_kinds.default_enabled() {
            return Err(
                "otel-http action_kinds.default must be false; enable action kinds explicitly"
                    .to_string(),
            );
        }
        // A client cert without its key (or vice versa) is never a safe default:
        // fail loud rather than silently drop the client identity.
        if self.tls.client_cert_path.is_some() != self.tls.client_key_path.is_some() {
            return Err(
                "otel-http tls_client_cert_path and tls_client_key_path must be set together"
                    .to_string(),
            );
        }
        // TLS material on a plaintext endpoint is a misconfiguration: it would be
        // silently ignored and give a false sense of security.
        if !endpoint.secure && self.tls.any_set() {
            return Err("otel-http TLS options require an https:// endpoint".to_string());
        }
        if !endpoint.secure && !self.allow_insecure {
            return Err("otel-http plaintext endpoint requires allow_insecure = true".to_string());
        }
        Ok(())
    }

    pub fn connect_timeout(&self) -> Duration {
        Duration::from_millis(u64::from(self.connect_timeout_ms))
    }

    pub fn request_timeout(&self) -> Duration {
        Duration::from_millis(u64::from(self.request_timeout_ms))
    }

    pub fn retry_backoff(&self) -> Duration {
        Duration::from_millis(u64::from(self.retry_backoff_ms))
    }

    pub fn shutdown_flush_deadline(&self) -> Duration {
        Duration::from_millis(u64::from(self.shutdown_flush_deadline_ms))
    }
}

pub fn parse_otel_http_plugin_config(raw: &str) -> Result<OtelHttpExporterConfig, String> {
    let value = raw
        .parse::<toml::Value>()
        .map_err(|error| format!("parse otel-http plugin config: {error}"))?;
    let table = value
        .as_table()
        .ok_or_else(|| "otel-http plugin config must be a TOML table".to_string())?;

    let action_kinds = table
        .get("action_kinds")
        .ok_or_else(|| "missing config key plugin.otel-http.action_kinds".to_string())?
        .as_table()
        .ok_or_else(|| "plugin.otel-http.action_kinds must be a table".to_string())?;
    let action_kinds = SemanticActionKindSelection::from_config_entries(
        action_kinds
            .iter()
            .map(|(key, value)| {
                value
                    .as_bool()
                    .map(|enabled| (key.clone(), enabled))
                    .ok_or_else(|| format!("otel-http config action_kinds.{key} must be a bool"))
            })
            .collect::<Result<Vec<_>, _>>()?,
    )
    .map_err(|message| format!("invalid plugin.otel-http.action_kinds: {message}"))?;

    let headers = match table.get("headers") {
        Some(headers) => parse_headers_array(headers.as_array().ok_or_else(|| {
            "plugin.otel-http.headers must be an array of name/value entries".to_string()
        })?)?,
        None => Vec::new(),
    };

    let mut entries = Vec::with_capacity(table.len().saturating_sub(2));
    for (key, value) in table {
        if key == "action_kinds" || key == "headers" {
            continue;
        }
        let value = match value {
            toml::Value::String(value) => value.clone(),
            toml::Value::Integer(value) => value.to_string(),
            toml::Value::Boolean(value) => value.to_string(),
            _ => {
                return Err(format!(
                    "otel-http config {key} must be a string, integer, or boolean"
                ));
            }
        };
        entries.push((key.clone(), value));
    }

    let mut config = OtelHttpExporterConfig::parse_section("plugin.otel-http", entries)?;
    config.action_kinds = action_kinds;
    config.headers = headers;
    config.validate_enabled_route()?;
    Ok(config)
}

/// Headers the transport derives from the endpoint, encoding and body. A config
/// that overrides one of these would silently break request framing, so a
/// collision is a hard configuration error rather than a last-write-wins.
const RESERVED_HEADERS: [&str; 6] = [
    "host",
    "content-length",
    "content-type",
    "content-encoding",
    "connection",
    "transfer-encoding",
];

fn parse_headers_array(entries: &[toml::Value]) -> Result<Vec<(String, String)>, String> {
    let mut headers: Vec<(String, String)> = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        let entry = entry.as_table().ok_or_else(|| {
            format!("otel-http config headers[{index}] must be a name/value table")
        })?;
        let name = header_entry_field(entry, index, "name")?;
        let value = header_entry_field(entry, index, "value")?;
        if let Some(unknown) = entry.keys().find(|key| *key != "name" && *key != "value") {
            return Err(format!(
                "unknown config key plugin.otel-http.headers[{index}].{unknown}"
            ));
        }
        validate_header_name(name)?;
        validate_header_value(name, value)?;
        // A header configured twice has no single correct meaning, and keeping
        // one silently would leave an operator looking at a value that is not
        // being sent. HTTP field names are case-insensitive.
        if headers
            .iter()
            .any(|(seen, _)| seen.eq_ignore_ascii_case(name))
        {
            return Err(format!(
                "otel-http header {name:?} is configured more than once"
            ));
        }
        headers.push((name.to_string(), value.to_string()));
    }
    Ok(headers)
}

fn header_entry_field<'a>(
    entry: &'a toml::Table,
    index: usize,
    key: &str,
) -> Result<&'a str, String> {
    entry
        .get(key)
        .ok_or_else(|| format!("missing config key plugin.otel-http.headers[{index}].{key}"))?
        .as_str()
        .ok_or_else(|| format!("otel-http config headers[{index}].{key} must be a string"))
}

fn validate_header_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("otel-http config headers has an empty header name".to_string());
    }
    // RFC 7230 token: anything else can terminate the name early and forge a
    // second header or request line.
    if !name
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte))
    {
        return Err(format!(
            "invalid otel-http header name {name:?}: must be an RFC 7230 token"
        ));
    }
    if RESERVED_HEADERS.contains(&name.to_ascii_lowercase().as_str()) {
        return Err(format!(
            "otel-http header {name:?} is reserved: the exporter derives it from \
             endpoint, encoding and body"
        ));
    }
    Ok(())
}

fn validate_header_value(name: &str, value: &str) -> Result<(), String> {
    if let Some(index) = value.find(['\r', '\n', '\0']) {
        return Err(format!(
            "invalid otel-http header value for {name:?}: control character at byte {index} \
             would split the request"
        ));
    }
    Ok(())
}

/// Parsed `http(s)://host[:port]/path` collector endpoint.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Endpoint {
    pub host: String,
    pub port: u16,
    pub path: String,
    /// `true` for `https://` — the transport must wrap the socket in TLS.
    pub secure: bool,
}

impl Endpoint {
    pub fn parse(url: &str) -> Result<Self, String> {
        let (rest, secure) = if let Some(rest) = url.strip_prefix("https://") {
            (rest, true)
        } else if let Some(rest) = url.strip_prefix("http://") {
            (rest, false)
        } else {
            return Err(format!(
                "invalid otel-http endpoint {url}: must start with http:// or https://"
            ));
        };
        let (authority, path) = match rest.find('/') {
            Some(index) => (&rest[..index], &rest[index..]),
            None => (rest, "/v1/traces"),
        };
        let (host, port) = match authority.rsplit_once(':') {
            Some((host, port)) => {
                let port = port.parse::<u16>().map_err(|error| {
                    format!("invalid otel-http endpoint port in {url}: {error}")
                })?;
                (host, port)
            }
            None => (authority, 4318),
        };
        if host.is_empty() {
            return Err(format!("invalid otel-http endpoint {url}: missing host"));
        }
        Ok(Self {
            host: host.to_string(),
            port,
            path: path.to_string(),
            secure,
        })
    }

    pub fn authority(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

struct OtelHttpConfigValues {
    section_name: String,
    values: BTreeMap<String, String>,
}

impl OtelHttpConfigValues {
    fn from_entries(section_name: &str, entries: Vec<(String, String)>) -> Result<Self, String> {
        let mut values = BTreeMap::new();
        for (key, value) in entries {
            reject_unknown_key(section_name, &key)?;
            if values.insert(key.clone(), value).is_some() {
                return Err(format!("duplicate config key {section_name}.{key}"));
            }
        }
        Ok(Self {
            section_name: section_name.to_string(),
            values,
        })
    }

    fn optional(&self, key: &'static str) -> Option<String> {
        self.values
            .get(key)
            .filter(|value| !value.is_empty())
            .cloned()
    }

    fn required(&self, key: &'static str) -> Result<String, String> {
        self.values
            .get(key)
            .cloned()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("missing config key {}.{key}", self.section_name))
    }

    fn required_positive_u32(&self, key: &'static str) -> Result<u32, String> {
        let value = self
            .required(key)?
            .parse::<u32>()
            .map_err(|error| format!("invalid {}.{key}: {error}", self.section_name))?;
        if value == u32::default() {
            return Err(format!(
                "invalid {}.{key}: value must be positive",
                self.section_name
            ));
        }
        Ok(value)
    }
}

fn reject_unknown_key(section_name: &str, key: &str) -> Result<(), String> {
    match key {
        "endpoint"
        | "allow_insecure"
        | "queue_capacity"
        | "batch_max_spans"
        | "batch_timeout_ms"
        | "connect_timeout_ms"
        | "request_timeout_ms"
        | "retry_max_attempts"
        | "retry_backoff_ms"
        | "shutdown_flush_deadline_ms"
        | "tls_ca_cert_path"
        | "tls_client_cert_path"
        | "tls_client_key_path"
        | "encoding"
        | "compression"
        | "attribute_mode" => Ok(()),
        _ => Err(format!("unknown config key {section_name}.{key}")),
    }
}

#[cfg(test)]
mod tests {
    use semantic_action::SemanticActionKind;

    use super::parse_otel_http_plugin_config;

    #[test]
    fn official_template_enables_tool_graph_actions_and_parses() {
        let raw = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../../examples/plugins/builtin/otel-http/otel-http.config.toml"
        ));

        let config = parse_otel_http_plugin_config(raw).expect("official template parses");

        assert!(config.action_kinds.enabled(SemanticActionKind::LlmToolCall));
        assert!(
            config
                .action_kinds
                .enabled(SemanticActionKind::LlmToolResult)
        );
        assert!(
            config
                .action_kinds
                .enabled(SemanticActionKind::AgentInvocation)
        );
    }
}
