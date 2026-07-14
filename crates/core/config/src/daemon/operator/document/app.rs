use super::*;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct ApplicationDocument {
    pub enabled: bool,
    pub http1_enabled: bool,
    pub http2_enabled: bool,
    pub http: HttpApplicationDocument,
    pub http2: Http2ApplicationDocument,
}

impl Default for ApplicationDocument {
    fn default() -> Self {
        Self {
            enabled: true,
            http1_enabled: true,
            http2_enabled: true,
            http: HttpApplicationDocument::default(),
            http2: Http2ApplicationDocument::default(),
        }
    }
}

impl ApplicationDocument {
    pub(super) fn from_config(config: &ApplicationProtocolConfig) -> Self {
        Self {
            enabled: config.enabled,
            http1_enabled: config.http1_enabled,
            http2_enabled: config.http2_enabled,
            http: HttpApplicationDocument {
                capture_host: config.capture_host,
                sse_enabled: config.sse_enabled,
                sse_data_policy: sse_data_policy_as_str(config.sse_data_policy).to_string(),
                sse_max_buffer_bytes: config.sse_max_buffer_bytes,
                sse_max_data_bytes: config.sse_max_data_bytes,
            },
            http2: Http2ApplicationDocument {
                max_frame_bytes: config.http2_max_frame_bytes,
                max_connection_buffer_bytes: config.http2_max_connection_buffer_bytes,
                emit_data_preview: config.http2_emit_data_preview,
                max_data_preview_bytes: config.http2_max_data_preview_bytes,
            },
        }
    }

    pub(super) fn to_config(&self) -> Result<ApplicationProtocolConfig, String> {
        Ok(ApplicationProtocolConfig {
            enabled: self.enabled,
            http1_enabled: self.http1_enabled,
            http2_enabled: self.http2_enabled,
            capture_host: self.http.capture_host,
            sse_enabled: self.http.sse_enabled,
            sse_data_policy: parse_value(
                "application.http.sse_data_policy",
                &self.http.sse_data_policy,
            )?,
            sse_max_buffer_bytes: require_positive_u64(
                "application.http.sse_max_buffer_bytes",
                self.http.sse_max_buffer_bytes,
            )?,
            sse_max_data_bytes: require_positive_u64(
                "application.http.sse_max_data_bytes",
                self.http.sse_max_data_bytes,
            )?,
            http2_max_frame_bytes: require_positive_u64(
                "application.http2.max_frame_bytes",
                self.http2.max_frame_bytes,
            )?,
            http2_max_connection_buffer_bytes: require_positive_u64(
                "application.http2.max_connection_buffer_bytes",
                self.http2.max_connection_buffer_bytes,
            )?,
            http2_emit_data_preview: self.http2.emit_data_preview,
            http2_max_data_preview_bytes: require_positive_u64(
                "application.http2.max_data_preview_bytes",
                self.http2.max_data_preview_bytes,
            )?,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct HttpApplicationDocument {
    pub capture_host: bool,
    pub sse_enabled: bool,
    pub sse_data_policy: String,
    pub sse_max_buffer_bytes: u64,
    pub sse_max_data_bytes: u64,
}

impl Default for HttpApplicationDocument {
    fn default() -> Self {
        Self {
            capture_host: true,
            sse_enabled: true,
            sse_data_policy: "preview".to_string(),
            sse_max_buffer_bytes: 4194304,
            sse_max_data_bytes: 65536,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct Http2ApplicationDocument {
    pub max_frame_bytes: u64,
    pub max_connection_buffer_bytes: u64,
    pub emit_data_preview: bool,
    pub max_data_preview_bytes: u64,
}

impl Default for Http2ApplicationDocument {
    fn default() -> Self {
        Self {
            max_frame_bytes: 65536,
            max_connection_buffer_bytes: 4194304,
            emit_data_preview: true,
            max_data_preview_bytes: 65536,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct ResourceMetricsDocument {
    pub enabled: bool,
    pub interval_ms: u64,
    pub include_children: bool,
    pub include_system: bool,
    pub cpu_alert_percent_millis: String,
    pub memory_alert_rss_kb: String,
}

impl Default for ResourceMetricsDocument {
    fn default() -> Self {
        Self {
            enabled: true,
            interval_ms: 1000,
            include_children: true,
            include_system: true,
            cpu_alert_percent_millis: "disabled".to_string(),
            memory_alert_rss_kb: "disabled".to_string(),
        }
    }
}

impl ResourceMetricsDocument {
    pub(super) fn to_config(&self) -> Result<ResourceMetricsConfig, String> {
        Ok(ResourceMetricsConfig {
            enabled: self.enabled,
            interval_ms: require_positive_u64("resource_metrics.interval_ms", self.interval_ms)?,
            include_children: self.include_children,
            include_system: self.include_system,
            cpu_alert_percent_millis: parse_disabled_or_positive_u64(
                "resource_metrics.cpu_alert_percent_millis",
                &self.cpu_alert_percent_millis,
            )?,
            memory_alert_rss_kb: parse_disabled_or_positive_u64(
                "resource_metrics.memory_alert_rss_kb",
                &self.memory_alert_rss_kb,
            )?,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct ProviderDocument {
    pub rules_enabled: bool,
    pub rules_path: String,
    pub unknown_provider_label: String,
}

impl Default for ProviderDocument {
    fn default() -> Self {
        Self {
            rules_enabled: false,
            rules_path: "/etc/actrail/provider-rules.conf".to_string(),
            unknown_provider_label: "unknown".to_string(),
        }
    }
}

impl ProviderDocument {
    pub(super) fn to_config(&self) -> Option<ProviderRuleSetConfig> {
        self.rules_enabled.then(|| ProviderRuleSetConfig {
            rules_path: PathBuf::from(&self.rules_path),
            unknown_provider_label: self.unknown_provider_label.clone(),
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct EnforcementDocument {
    pub enabled: bool,
    pub backend: String,
    pub scope: String,
    pub rules_path: String,
    pub builtin_rules: Vec<EnforcementBuiltinRuleDocument>,
    pub default_decision: String,
    pub mark_strategy: String,
    pub audit_enabled: bool,
    pub event_buffer_bytes: u32,
}

impl Default for EnforcementDocument {
    fn default() -> Self {
        Self {
            enabled: false,
            backend: "fanotify".to_string(),
            scope: "trace".to_string(),
            rules_path: "/etc/actrail/enforcement-rules.conf".to_string(),
            builtin_rules: Vec::new(),
            default_decision: "allow".to_string(),
            mark_strategy: "parent-directories".to_string(),
            audit_enabled: true,
            event_buffer_bytes: 65536,
        }
    }
}

impl EnforcementDocument {
    pub(super) fn to_config(&self) -> Result<EnforcementConfig, String> {
        Ok(EnforcementConfig {
            enabled: self.enabled,
            backend: parse_value("enforcement.backend", &self.backend)?,
            scope: parse_value("enforcement.scope", &self.scope)?,
            rules_path: PathBuf::from(&self.rules_path),
            builtin_rules: self
                .builtin_rules
                .iter()
                .map(EnforcementBuiltinRuleDocument::to_config)
                .collect::<Result<Vec<_>, _>>()?,
            default_decision: parse_value("enforcement.default_decision", &self.default_decision)?,
            mark_strategy: parse_value("enforcement.mark_strategy", &self.mark_strategy)?,
            audit_enabled: self.audit_enabled,
            event_buffer_bytes: require_positive_u32(
                "enforcement.event_buffer_bytes",
                self.event_buffer_bytes,
            )?,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct EnforcementBuiltinRuleDocument {
    pub rule_id: String,
    pub path: String,
}

impl Default for EnforcementBuiltinRuleDocument {
    fn default() -> Self {
        Self {
            rule_id: String::new(),
            path: String::new(),
        }
    }
}

impl EnforcementBuiltinRuleDocument {
    pub(super) fn from_config(config: &EnforcementBuiltinRuleConfig) -> Self {
        Self {
            rule_id: config.rule_id.clone(),
            path: config.path.clone(),
        }
    }

    fn to_config(&self) -> Result<EnforcementBuiltinRuleConfig, String> {
        if self.rule_id.trim().is_empty() {
            return Err("enforcement.builtin_rules.rule_id must not be empty".to_string());
        }
        if self.path.trim().is_empty() {
            return Err("enforcement.builtin_rules.path must not be empty".to_string());
        }
        Ok(EnforcementBuiltinRuleConfig {
            rule_id: self.rule_id.clone(),
            path: self.path.clone(),
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct SupervisionDocument {
    pub startup_wait_ms: u64,
    pub shutdown_wait_ms: u64,
    pub poll_interval_ms: u64,
}

impl Default for SupervisionDocument {
    fn default() -> Self {
        Self {
            startup_wait_ms: 30_000,
            shutdown_wait_ms: 5000,
            poll_interval_ms: 100,
        }
    }
}
