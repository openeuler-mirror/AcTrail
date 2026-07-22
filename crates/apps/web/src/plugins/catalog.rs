use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use config_core::daemon::OperatorConfig;
use control_contract::command::{
    ControlCommand, PluginCommandCommand, PluginConfigGetCommand, PluginConfigUpdateCommand,
    PluginConfigValidateCommand, PluginListCommand, PluginLoadCommand, PluginUnloadCommand,
};
use control_contract::reply::{
    ControlReply, PluginCommandReply, PluginConfigReply, PluginConfigValidationReply,
};
use model_core::ids::RequestId;
use plugin_system::{FilePolicyDecision, PluginHostGrant, PluginHostGrants, PluginInstanceStatus};
use serde::Deserialize;
use uds_control_client::{UdsControlClient, UdsSocketTransport};

use super::package::{InstalledPackage, PluginDirectory};

pub(crate) struct InstalledPluginCatalog {
    config_path: Option<PathBuf>,
    socket_path: PathBuf,
    directory: PluginDirectory,
}

pub(crate) struct CatalogSnapshot {
    pub(super) config_path: Option<PathBuf>,
    pub(super) directory: PathBuf,
    pub(super) packages: Vec<CatalogPackage>,
    pub(super) runtime_plugins: Vec<PluginInstanceStatus>,
    pub(super) runtime_error: Option<String>,
}

pub(super) struct CatalogPackage {
    pub(super) package: InstalledPackage,
    pub(super) loaded_instances: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PluginLoadOptions {
    instance_id: String,
    #[serde(default)]
    grants: PluginGrantOptions,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct PluginGrantOptions {
    #[serde(default)]
    file_policy_rules_apply: Vec<FilePolicyApplyGrantOption>,
    #[serde(default)]
    env_read: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FilePolicyApplyGrantOption {
    decision: String,
    path_scope: String,
}

impl InstalledPluginCatalog {
    pub(crate) fn new(config_path: Option<&Path>, config: &OperatorConfig) -> Result<Self, String> {
        Ok(Self {
            config_path: config_path.map(Path::to_path_buf),
            socket_path: config.socket_path.clone(),
            directory: PluginDirectory::new(&config.plugin_discovery)?,
        })
    }

    pub(crate) fn refresh(&self) -> Result<CatalogSnapshot, String> {
        let packages = self.directory.scan()?;
        let runtime = self.list_runtime_plugins();
        let (runtime_plugins, runtime_error) = match runtime {
            Ok(plugins) => (plugins, None),
            Err(error) => (Vec::new(), Some(error)),
        };
        let packages = packages
            .into_iter()
            .map(|package| {
                let loaded_instances = runtime_error.is_none().then(|| {
                    package
                        .plugin_id
                        .as_deref()
                        .map(|plugin_id| {
                            runtime_plugins
                                .iter()
                                .filter(|status| status.plugin_id == plugin_id)
                                .map(|status| status.instance_id.clone())
                                .collect()
                        })
                        .unwrap_or_default()
                });
                CatalogPackage {
                    package,
                    loaded_instances,
                }
            })
            .collect();
        Ok(CatalogSnapshot {
            config_path: self.config_path.clone(),
            directory: self.directory.root().to_path_buf(),
            packages,
            runtime_plugins,
            runtime_error,
        })
    }

    pub(crate) fn load(
        &self,
        package_key: &str,
        options: PluginLoadOptions,
    ) -> Result<PluginInstanceStatus, String> {
        if package_key.trim().is_empty() {
            return Err("plugin package key must not be empty".to_string());
        }
        let package = self
            .directory
            .scan()?
            .into_iter()
            .find(|package| package.key == package_key)
            .ok_or_else(|| format!("discovered plugin package {package_key} was not found"))?;
        if !package.activation_ready() {
            return Err(package.issue.unwrap_or_else(|| {
                format!("discovered plugin package {package_key} is not loadable")
            }));
        }
        let plugin_id = package
            .plugin_id
            .as_deref()
            .ok_or_else(|| format!("discovered plugin package {package_key} has no plugin id"))?;
        let instance_id = options.instance_id.trim();
        if instance_id.is_empty() || instance_id != options.instance_id {
            return Err(
                "plugin instance id must be non-empty and have no surrounding whitespace"
                    .to_string(),
            );
        }
        let host_grants = options.grants.resolve(&package)?;
        let loaded = self
            .list_runtime_plugins()?
            .into_iter()
            .filter(|status| status.plugin_id == plugin_id)
            .map(|status| status.instance_id)
            .collect::<Vec<_>>();
        if !loaded.is_empty() {
            return Err(format!(
                "plugin {plugin_id} is already loaded as {}",
                loaded.join(", ")
            ));
        }
        let manifest_path = package
            .manifest_path
            .as_ref()
            .ok_or_else(|| format!("discovered plugin package {package_key} has no manifest"))?;
        let mut client = self.client();
        let reply = client
            .send(ControlCommand::PluginLoad(PluginLoadCommand {
                request_id: self.request_id()?,
                manifest_path: manifest_path.display().to_string(),
                plugin_config_path: package
                    .plugin_config_path
                    .as_ref()
                    .map(|path| path.display().to_string()),
                instance_id: instance_id.to_string(),
                host_grants,
            }))
            .map_err(|error| {
                format!(
                    "daemon plugin load failed: {}: {}",
                    error.code, error.message
                )
            })?;
        match reply {
            ControlReply::PluginStatus(status) => Ok(status),
            _ => Err("daemon returned unexpected reply for plugin load".to_string()),
        }
    }

    pub(crate) fn unload(&self, instance_id: &str) -> Result<PluginInstanceStatus, String> {
        if instance_id.trim().is_empty() {
            return Err("plugin instance id must not be empty".to_string());
        }
        let mut client = self.client();
        let reply = client
            .send(ControlCommand::PluginUnload(PluginUnloadCommand {
                request_id: self.request_id()?,
                instance_id: instance_id.to_string(),
            }))
            .map_err(|error| {
                format!(
                    "daemon plugin unload failed: {}: {}",
                    error.code, error.message
                )
            })?;
        match reply {
            ControlReply::PluginStatus(status) => Ok(status),
            _ => Err("daemon returned unexpected reply for plugin unload".to_string()),
        }
    }

    pub(crate) fn command(
        &self,
        instance_id: &str,
        argv: Vec<String>,
    ) -> Result<PluginCommandReply, String> {
        if instance_id.trim().is_empty() {
            return Err("plugin instance id must not be empty".to_string());
        }
        if argv.is_empty() {
            return Err("plugin command argv must not be empty".to_string());
        }
        let mut client = self.client();
        let reply = client
            .send(ControlCommand::PluginCommand(PluginCommandCommand {
                request_id: self.request_id()?,
                instance_id: instance_id.to_string(),
                argv,
            }))
            .map_err(|error| {
                format!(
                    "daemon plugin command failed: {}: {}",
                    error.code, error.message
                )
            })?;
        match reply {
            ControlReply::PluginCommand(reply) => Ok(reply),
            _ => Err("daemon returned unexpected reply for plugin command".to_string()),
        }
    }

    pub(crate) fn config(&self, instance_id: &str) -> Result<PluginConfigReply, String> {
        let mut client = self.client();
        let reply = client
            .send(ControlCommand::PluginConfigGet(PluginConfigGetCommand {
                request_id: self.request_id()?,
                instance_id: instance_id.to_string(),
            }))
            .map_err(Self::plugin_config_error)?;
        match reply {
            ControlReply::PluginConfig(reply) => Ok(reply),
            _ => Err("daemon returned unexpected reply for plugin config query".to_string()),
        }
    }

    pub(crate) fn validate_config(
        &self,
        instance_id: &str,
        config_json: String,
    ) -> Result<PluginConfigValidationReply, String> {
        let mut client = self.client();
        let reply = client
            .send(ControlCommand::PluginConfigValidate(
                PluginConfigValidateCommand {
                    request_id: self.request_id()?,
                    instance_id: instance_id.to_string(),
                    config_json,
                },
            ))
            .map_err(Self::plugin_config_error)?;
        match reply {
            ControlReply::PluginConfigValidation(reply) => Ok(reply),
            _ => Err("daemon returned unexpected reply for plugin config validation".to_string()),
        }
    }

    pub(crate) fn update_config(
        &self,
        instance_id: &str,
        config_json: String,
    ) -> Result<PluginConfigReply, String> {
        let mut client = self.client();
        let reply = client
            .send(ControlCommand::PluginConfigUpdate(
                PluginConfigUpdateCommand {
                    request_id: self.request_id()?,
                    instance_id: instance_id.to_string(),
                    config_json,
                },
            ))
            .map_err(Self::plugin_config_error)?;
        match reply {
            ControlReply::PluginConfig(reply) => Ok(reply),
            _ => Err("daemon returned unexpected reply for plugin config update".to_string()),
        }
    }

    fn list_runtime_plugins(&self) -> Result<Vec<PluginInstanceStatus>, String> {
        let mut client = self.client();
        let reply = client
            .send(ControlCommand::PluginList(PluginListCommand {
                request_id: self.request_id()?,
            }))
            .map_err(|error| {
                format!(
                    "daemon plugin status unavailable: {}: {}",
                    error.code, error.message
                )
            })?;
        match reply {
            ControlReply::PluginList(statuses) => Ok(statuses),
            _ => Err("daemon returned unexpected reply for plugin list".to_string()),
        }
    }

    fn client(&self) -> UdsControlClient<UdsSocketTransport> {
        UdsControlClient::new(UdsSocketTransport::new(self.socket_path.clone()))
    }

    fn request_id(&self) -> Result<RequestId, String> {
        let duration = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "system clock is before unix epoch; cannot build control request id")?;
        let micros = u64::try_from(duration.as_micros())
            .map_err(|_| "system clock micros overflowed control request id")?;
        Ok(RequestId::new(micros))
    }

    fn plugin_config_error(error: control_contract::reply::ControlError) -> String {
        format!(
            "daemon plugin config failed: {}: {}",
            error.code, error.message
        )
    }
}

impl PluginGrantOptions {
    fn resolve(self, package: &InstalledPackage) -> Result<Vec<String>, String> {
        let needs_file_policy = package
            .parameterized_host_grants
            .iter()
            .any(|capability| capability == "file-policy.rules.apply");
        let needs_env_read = package
            .parameterized_host_grants
            .iter()
            .any(|capability| capability == "env-read");
        if needs_file_policy && self.file_policy_rules_apply.is_empty() {
            return Err(
                "file-policy.rules.apply requires at least one decision and path scope".to_string(),
            );
        }
        if !needs_file_policy && !self.file_policy_rules_apply.is_empty() {
            return Err("plugin did not request file-policy.rules.apply".to_string());
        }
        if needs_env_read && self.env_read.is_empty() {
            return Err("env-read requires at least one environment variable name".to_string());
        }
        if !needs_env_read && !self.env_read.is_empty() {
            return Err("plugin did not request env-read".to_string());
        }

        let mut values = package.automatic_host_grants.clone();
        for option in self.file_policy_rules_apply {
            let decision = FilePolicyDecision::from_wire(&option.decision)?;
            values.push(
                PluginHostGrant::FilePolicyRulesApply {
                    decision,
                    path: option.path_scope,
                }
                .to_wire(),
            );
        }
        values.extend(
            self.env_read
                .into_iter()
                .map(|name| PluginHostGrant::EnvRead { name }.to_wire()),
        );
        PluginHostGrants::parse(&values)?;
        Ok(values)
    }
}
