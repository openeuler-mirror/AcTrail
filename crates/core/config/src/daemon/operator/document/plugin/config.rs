use super::super::*;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(in super::super) struct PluginsDocument {
    discovery: PluginDiscoveryDocument,
    alerts: PluginAlertRuntimeDocument,
    startup: StartupPluginsDocument,
}

impl Default for PluginsDocument {
    fn default() -> Self {
        Self {
            discovery: PluginDiscoveryDocument::default(),
            alerts: PluginAlertRuntimeDocument::default(),
            startup: StartupPluginsDocument::default(),
        }
    }
}

impl PluginsDocument {
    pub(in super::super) fn from_config(
        discovery: &PluginDiscoveryConfig,
        alerts: &PluginAlertRuntimeConfig,
        startup: &StartupPluginsConfig,
    ) -> Self {
        Self {
            discovery: PluginDiscoveryDocument::from_config(discovery),
            alerts: PluginAlertRuntimeDocument::from_config(alerts),
            startup: StartupPluginsDocument::from_config(startup),
        }
    }

    pub(in super::super) fn to_config(
        &self,
    ) -> Result<
        (
            PluginDiscoveryConfig,
            PluginAlertRuntimeConfig,
            StartupPluginsConfig,
        ),
        String,
    > {
        Ok((
            self.discovery.to_config()?,
            self.alerts.to_config()?,
            self.startup.to_config()?,
        ))
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
struct PluginAlertRuntimeDocument {
    queue_capacity: u32,
    writes_per_cycle: u32,
    drain_timeout_ms: u64,
    webhook: Option<PluginAlertWebhookDocument>,
}

impl Default for PluginAlertRuntimeDocument {
    fn default() -> Self {
        Self {
            queue_capacity: DEFAULT_PLUGIN_ALERT_QUEUE_CAPACITY,
            writes_per_cycle: DEFAULT_PLUGIN_ALERT_WRITES_PER_CYCLE,
            drain_timeout_ms: DEFAULT_PLUGIN_ALERT_DRAIN_TIMEOUT_MS,
            webhook: None,
        }
    }
}

impl PluginAlertRuntimeDocument {
    fn from_config(config: &PluginAlertRuntimeConfig) -> Self {
        Self {
            queue_capacity: config.queue_capacity,
            writes_per_cycle: config.writes_per_cycle,
            drain_timeout_ms: config.drain_timeout_ms,
            webhook: config
                .webhook
                .as_ref()
                .map(PluginAlertWebhookDocument::from_config),
        }
    }

    fn to_config(&self) -> Result<PluginAlertRuntimeConfig, String> {
        let queue_capacity =
            require_positive_u32("plugins.alerts.queue_capacity", self.queue_capacity)?;
        let writes_per_cycle =
            require_positive_u32("plugins.alerts.writes_per_cycle", self.writes_per_cycle)?;
        if writes_per_cycle > queue_capacity {
            return Err(
                "plugins.alerts.writes_per_cycle must not exceed queue_capacity".to_string(),
            );
        }
        let webhook = self
            .webhook
            .as_ref()
            .map(PluginAlertWebhookDocument::to_config)
            .transpose()?;
        Ok(PluginAlertRuntimeConfig {
            queue_capacity,
            writes_per_cycle,
            drain_timeout_ms: require_positive_u64(
                "plugins.alerts.drain_timeout_ms",
                self.drain_timeout_ms,
            )?,
            webhook,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
struct PluginAlertWebhookDocument {
    enabled: bool,
    url: String,
    auth_token: String,
    timeout_ms: u64,
    redact_sensitive_body: bool,
}

impl Default for PluginAlertWebhookDocument {
    fn default() -> Self {
        Self {
            enabled: false,
            url: String::new(),
            auth_token: String::new(),
            timeout_ms: 5_000,
            redact_sensitive_body: true,
        }
    }
}

impl PluginAlertWebhookDocument {
    fn from_config(config: &AlertWebhookConfig) -> Self {
        Self {
            enabled: config.enabled,
            url: config.url.clone(),
            auth_token: config.auth_token.clone(),
            timeout_ms: config.timeout_ms,
            redact_sensitive_body: config.redact_sensitive_body,
        }
    }

    fn to_config(&self) -> Result<AlertWebhookConfig, String> {
        if !self.enabled {
            return Ok(AlertWebhookConfig {
                enabled: false,
                url: String::new(),
                auth_token: String::new(),
                timeout_ms: 0,
                redact_sensitive_body: true,
            });
        }
        let url = required_non_empty("plugins.alerts.webhook.url", &self.url)?.to_string();
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(
                "plugins.alerts.webhook.url must start with http:// or https://".to_string(),
            );
        }
        Ok(AlertWebhookConfig {
            enabled: true,
            url,
            auth_token: self.auth_token.clone(),
            timeout_ms: require_positive_u64("plugins.alerts.webhook.timeout_ms", self.timeout_ms)?,
            redact_sensitive_body: self.redact_sensitive_body,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct PluginDiscoveryDocument {
    pub directory: String,
    pub max_packages: u32,
    pub manifest_max_bytes: u64,
}

impl Default for PluginDiscoveryDocument {
    fn default() -> Self {
        Self {
            directory: DEFAULT_PLUGIN_DISCOVERY_DIRECTORY.to_string(),
            max_packages: DEFAULT_PLUGIN_DISCOVERY_MAX_PACKAGES,
            manifest_max_bytes: DEFAULT_PLUGIN_DISCOVERY_MANIFEST_MAX_BYTES,
        }
    }
}

impl PluginDiscoveryDocument {
    fn from_config(config: &PluginDiscoveryConfig) -> Self {
        Self {
            directory: config.directory.display().to_string(),
            max_packages: config.max_packages,
            manifest_max_bytes: config.manifest_max_bytes,
        }
    }

    fn to_config(&self) -> Result<PluginDiscoveryConfig, String> {
        let config = PluginDiscoveryConfig {
            directory: PathBuf::from(required_non_empty(
                "plugins.discovery.directory",
                &self.directory,
            )?),
            max_packages: require_positive_u32(
                "plugins.discovery.max_packages",
                self.max_packages,
            )?,
            manifest_max_bytes: require_positive_u64(
                "plugins.discovery.manifest_max_bytes",
                self.manifest_max_bytes,
            )?,
        };
        config.resolved_directory()?;
        Ok(config)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct StartupPluginsDocument {
    pub enabled: bool,
    pub failure_policy: String,
    pub load: Vec<StartupPluginLoadDocument>,
}

impl Default for StartupPluginsDocument {
    fn default() -> Self {
        Self {
            enabled: false,
            failure_policy: StartupPluginFailurePolicy::FailFast.as_str().to_string(),
            load: Vec::new(),
        }
    }
}

impl StartupPluginsDocument {
    fn from_config(config: &StartupPluginsConfig) -> Self {
        Self {
            enabled: config.enabled,
            failure_policy: config.failure_policy.as_str().to_string(),
            load: config
                .load
                .iter()
                .map(StartupPluginLoadDocument::from_config)
                .collect(),
        }
    }

    fn to_config(&self) -> Result<StartupPluginsConfig, String> {
        let failure_policy = parse_value::<StartupPluginFailurePolicy>(
            "plugins.startup.failure_policy",
            &self.failure_policy,
        )?;
        let load = self
            .load
            .iter()
            .map(StartupPluginLoadDocument::to_config)
            .collect::<Result<Vec<_>, _>>()?;
        if self.enabled && load.iter().all(|item| !item.enabled) {
            return Err(
                "plugins.startup.enabled=true requires at least one enabled load entry".to_string(),
            );
        }
        let mut seen = std::collections::BTreeSet::new();
        for item in load.iter().filter(|item| item.enabled) {
            if !seen.insert(item.instance_id.clone()) {
                return Err(format!(
                    "plugins.startup.load instance {} is duplicated",
                    item.instance_id
                ));
            }
        }
        Ok(StartupPluginsConfig {
            enabled: self.enabled,
            failure_policy,
            load,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct StartupPluginLoadDocument {
    pub instance: String,
    pub enabled: bool,
    pub failure_policy: String,
    pub manifest: String,
    pub plugin_config: String,
    pub host_grants: Vec<String>,
}

impl Default for StartupPluginLoadDocument {
    fn default() -> Self {
        Self {
            instance: String::new(),
            enabled: true,
            failure_policy: String::new(),
            manifest: String::new(),
            plugin_config: String::new(),
            host_grants: Vec::new(),
        }
    }
}

impl StartupPluginLoadDocument {
    fn from_config(config: &StartupPluginLoadConfig) -> Self {
        Self {
            instance: config.instance_id.clone(),
            enabled: config.enabled,
            failure_policy: config
                .failure_policy
                .map(StartupPluginFailurePolicy::as_str)
                .unwrap_or("")
                .to_string(),
            manifest: config.manifest_path.display().to_string(),
            plugin_config: config
                .plugin_config_path
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_default(),
            host_grants: config.host_grants.clone(),
        }
    }

    fn to_config(&self) -> Result<StartupPluginLoadConfig, String> {
        let failure_policy = if self.failure_policy.trim().is_empty() {
            None
        } else {
            Some(parse_value::<StartupPluginFailurePolicy>(
                "plugins.startup.load.failure_policy",
                &self.failure_policy,
            )?)
        };
        Ok(StartupPluginLoadConfig {
            instance_id: required_non_empty("plugins.startup.load.instance", &self.instance)?
                .to_string(),
            enabled: self.enabled,
            failure_policy,
            manifest_path: PathBuf::from(required_non_empty(
                "plugins.startup.load.manifest",
                &self.manifest,
            )?),
            plugin_config_path: if self.plugin_config.trim().is_empty() {
                None
            } else {
                Some(PathBuf::from(&self.plugin_config))
            },
            host_grants: self.host_grants.clone(),
        })
    }
}
