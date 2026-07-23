use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use control_contract::command::PluginLoadCommand;
use control_contract::reply::{ControlError, PluginConfigReply, PluginConfigValidationReply};
use plugin_system::PluginManifest;
use serde_json::Value;

#[derive(Default)]
pub(super) struct PluginConfigManager {
    loaded: BTreeMap<String, LoadedPluginConfig>,
}

pub(super) struct InspectedPluginConfig {
    pub(super) raw: Option<String>,
    loaded: LoadedPluginConfig,
}

impl InspectedPluginConfig {
    pub(super) fn load_command(&self) -> PluginLoadCommand {
        self.loaded.load_command.clone()
    }
}

#[derive(Clone)]
struct LoadedPluginConfig {
    load_command: PluginLoadCommand,
    plugin_id: String,
    format: String,
    schema: Option<Value>,
    config: Value,
    runtime_managed: bool,
}

impl PluginConfigManager {
    pub(super) fn inspect_load(
        command: &PluginLoadCommand,
        manifest_path: &Path,
        manifest: &PluginManifest,
    ) -> Result<InspectedPluginConfig, ControlError> {
        if manifest.plugin_config.required && command.plugin_config_path.is_none() {
            return Err(ControlError::new(
                "plugin_config",
                format!("plugin {} requires --plugin-config", manifest.id()),
            ));
        }
        let raw = command
            .plugin_config_path
            .as_ref()
            .map(|path| {
                fs::read_to_string(path).map_err(|error| {
                    ControlError::new("plugin_config", format!("read {path} failed: {error}"))
                })
            })
            .transpose()?;
        let schema = Self::read_schema(manifest_path, manifest)?;
        let config = raw
            .as_deref()
            .map(|raw| Self::parse_config(manifest.id(), &manifest.plugin_config.format, raw))
            .transpose()?
            .unwrap_or(Value::Null);
        if raw.is_some() {
            Self::validate_schema(manifest.id(), schema.as_ref(), &config)?;
        }
        Ok(InspectedPluginConfig {
            raw,
            loaded: LoadedPluginConfig {
                load_command: command.clone(),
                plugin_id: manifest.id().to_string(),
                format: manifest.plugin_config.format.clone(),
                schema,
                config,
                runtime_managed: manifest.plugin_config.runtime_managed,
            },
        })
    }

    pub(super) fn register(&mut self, inspected: InspectedPluginConfig) {
        self.loaded.insert(
            inspected.loaded.load_command.instance_id.clone(),
            inspected.loaded,
        );
    }

    pub(super) fn remove(&mut self, instance_id: &str) {
        self.loaded.remove(instance_id);
    }

    pub(super) fn document(&self, instance_id: &str) -> Result<PluginConfigReply, ControlError> {
        self.required(instance_id)?.document()
    }

    pub(super) fn runtime_managed(&self, instance_id: &str) -> Result<bool, ControlError> {
        Ok(self.required(instance_id)?.runtime_managed)
    }

    pub(super) fn runtime_document(
        &self,
        instance_id: &str,
        config_json: &str,
    ) -> Result<PluginConfigReply, ControlError> {
        let config = serde_json::from_str(config_json).map_err(|error| {
            ControlError::new(
                "plugin_config",
                format!("plugin returned invalid config JSON: {error}"),
            )
        })?;
        self.required(instance_id)?.document_with(&config)
    }

    pub(super) fn commit_runtime_config(
        &mut self,
        instance_id: &str,
        config_json: &str,
    ) -> Result<(), ControlError> {
        let config = serde_json::from_str(config_json).map_err(|error| {
            ControlError::new(
                "plugin_config",
                format!("plugin returned invalid config JSON: {error}"),
            )
        })?;
        let loaded = self.loaded.get_mut(instance_id).ok_or_else(|| {
            ControlError::new(
                "plugin_not_found",
                format!("plugin instance {instance_id} not found"),
            )
        })?;
        Self::validate_schema(&loaded.plugin_id, loaded.schema.as_ref(), &config)?;
        loaded.config = config;
        Ok(())
    }

    pub(super) fn validate(
        &self,
        instance_id: &str,
        config_json: &str,
    ) -> Result<PluginConfigValidationReply, ControlError> {
        let loaded = self.required(instance_id)?;
        loaded.require_editable()?;
        let errors = loaded.validation_errors(config_json, &loaded.config)?;
        Ok(PluginConfigValidationReply {
            instance_id: instance_id.to_string(),
            valid: errors.is_empty(),
            errors,
        })
    }

    pub(super) fn validate_runtime(
        &self,
        instance_id: &str,
        current_json: &str,
        config_json: &str,
    ) -> Result<PluginConfigValidationReply, ControlError> {
        let loaded = self.required(instance_id)?;
        loaded.require_editable()?;
        let current = serde_json::from_str(current_json).map_err(|error| {
            ControlError::new(
                "plugin_config",
                format!("plugin returned invalid config JSON: {error}"),
            )
        })?;
        let errors = loaded.validation_errors(config_json, &current)?;
        Ok(PluginConfigValidationReply {
            instance_id: instance_id.to_string(),
            valid: errors.is_empty(),
            errors,
        })
    }

    pub(super) fn prepare_update(
        &self,
        instance_id: &str,
        config_json: &str,
    ) -> Result<InspectedPluginConfig, ControlError> {
        let loaded = self.required(instance_id)?;
        loaded.require_editable()?;
        self.prepare_update_against(loaded, &loaded.config, config_json)
    }

    pub(super) fn prepare_runtime_update(
        &self,
        instance_id: &str,
        current_json: &str,
        config_json: &str,
    ) -> Result<InspectedPluginConfig, ControlError> {
        let loaded = self.required(instance_id)?;
        let current = serde_json::from_str(current_json).map_err(|error| {
            ControlError::new(
                "plugin_config",
                format!("plugin returned invalid config JSON: {error}"),
            )
        })?;
        self.prepare_update_against(loaded, &current, config_json)
    }

    fn prepare_update_against(
        &self,
        loaded: &LoadedPluginConfig,
        current: &Value,
        config_json: &str,
    ) -> Result<InspectedPluginConfig, ControlError> {
        loaded.require_editable()?;
        let errors = loaded.validation_errors(config_json, current)?;
        if !errors.is_empty() {
            return Err(ControlError::new(
                "plugin_config_validation",
                errors.join("; "),
            ));
        }
        let value = serde_json::from_str::<Value>(config_json).map_err(|error| {
            ControlError::new(
                "plugin_config_validation",
                format!("parse candidate JSON failed: {error}"),
            )
        })?;
        let raw = Self::serialize_config(&loaded.format, &value)?;
        let mut updated = loaded.clone();
        updated.config = value;
        Ok(InspectedPluginConfig {
            raw: Some(raw),
            loaded: updated,
        })
    }

    fn required(&self, instance_id: &str) -> Result<&LoadedPluginConfig, ControlError> {
        self.loaded.get(instance_id).ok_or_else(|| {
            ControlError::new(
                "plugin_not_found",
                format!("plugin instance {instance_id} not found"),
            )
        })
    }

    fn read_schema(
        manifest_path: &Path,
        manifest: &PluginManifest,
    ) -> Result<Option<Value>, ControlError> {
        let Some(schema_ref) = manifest.plugin_config.schema_ref.as_deref() else {
            return Ok(None);
        };
        if schema_ref.trim().is_empty() {
            return Err(ControlError::new(
                "plugin_config",
                format!("schema_ref for plugin {} must not be empty", manifest.id()),
            ));
        }
        let schema_path = Self::resolve_schema_path(manifest_path, schema_ref);
        let raw = fs::read_to_string(&schema_path).map_err(|error| {
            ControlError::new(
                "plugin_config",
                format!(
                    "read schema_ref {} for plugin {} failed: {error}",
                    schema_path.display(),
                    manifest.id()
                ),
            )
        })?;
        let schema = serde_json::from_str::<Value>(&raw).map_err(|error| {
            ControlError::new(
                "plugin_config",
                format!(
                    "parse JSON schema_ref {} for plugin {} failed: {error}",
                    schema_path.display(),
                    manifest.id()
                ),
            )
        })?;
        jsonschema::validator_for(&schema).map_err(|error| {
            ControlError::new(
                "plugin_config",
                format!(
                    "compile schema_ref {} for plugin {} failed: {error}",
                    schema_path.display(),
                    manifest.id()
                ),
            )
        })?;
        Ok(Some(schema))
    }

    fn parse_config(plugin_id: &str, format: &str, raw: &str) -> Result<Value, ControlError> {
        match format {
            "json" => serde_json::from_str(raw).map_err(|error| {
                ControlError::new(
                    "plugin_config",
                    format!("parse JSON config for plugin {plugin_id} failed: {error}"),
                )
            }),
            "toml" => {
                let config = toml::from_str::<toml::Value>(raw).map_err(|error| {
                    ControlError::new(
                        "plugin_config",
                        format!("parse TOML config for plugin {plugin_id} failed: {error}"),
                    )
                })?;
                serde_json::to_value(config).map_err(|error| {
                    ControlError::new(
                        "plugin_config",
                        format!("convert TOML config for plugin {plugin_id} failed: {error}"),
                    )
                })
            }
            other => Err(ControlError::new(
                "plugin_config",
                format!(
                    "unsupported format {other} for plugin {plugin_id}; supported formats: toml, json"
                ),
            )),
        }
    }

    fn serialize_config(format: &str, value: &Value) -> Result<String, ControlError> {
        match format {
            "json" => serde_json::to_string(value).map_err(|error| {
                ControlError::new("plugin_config", format!("serialize JSON config: {error}"))
            }),
            "toml" => toml::to_string(value).map_err(|error| {
                ControlError::new("plugin_config", format!("serialize TOML config: {error}"))
            }),
            other => Err(ControlError::new(
                "plugin_config",
                format!("unsupported plugin config format {other}"),
            )),
        }
    }

    fn validate_schema(
        plugin_id: &str,
        schema: Option<&Value>,
        value: &Value,
    ) -> Result<(), ControlError> {
        let Some(schema) = schema else {
            return Ok(());
        };
        let validator = jsonschema::validator_for(schema).map_err(|error| {
            ControlError::new(
                "plugin_config",
                format!("compile JSON schema for plugin {plugin_id} failed: {error}"),
            )
        })?;
        let errors = validator
            .iter_errors(value)
            .take(8)
            .map(|error| format!("{}: {error}", error.instance_path()))
            .collect::<Vec<_>>();
        if errors.is_empty() {
            Ok(())
        } else {
            Err(ControlError::new(
                "plugin_config_validation",
                errors.join("; "),
            ))
        }
    }

    fn resolve_schema_path(manifest_path: &Path, schema_ref: &str) -> PathBuf {
        let raw = PathBuf::from(schema_ref);
        if raw.is_absolute() {
            raw
        } else {
            manifest_path
                .parent()
                .map(|parent| parent.join(&raw))
                .unwrap_or(raw)
        }
    }
}

impl LoadedPluginConfig {
    fn editable(&self) -> bool {
        self.schema.is_some() && self.load_command.plugin_config_path.is_some()
    }

    fn require_editable(&self) -> Result<(), ControlError> {
        if self.editable() {
            Ok(())
        } else {
            Err(ControlError::new(
                "plugin_config_read_only",
                format!(
                    "plugin instance {} has no schema-backed editable config",
                    self.load_command.instance_id
                ),
            ))
        }
    }

    fn document(&self) -> Result<PluginConfigReply, ControlError> {
        self.document_with(&self.config)
    }

    fn document_with(&self, config: &Value) -> Result<PluginConfigReply, ControlError> {
        let config_json = serde_json::to_string(config).map_err(|error| {
            ControlError::new("plugin_config", format!("serialize config JSON: {error}"))
        })?;
        let schema_json = serde_json::to_string(self.schema.as_ref().unwrap_or(&Value::Null))
            .map_err(|error| {
                ControlError::new("plugin_config", format!("serialize schema JSON: {error}"))
            })?;
        Ok(PluginConfigReply {
            instance_id: self.load_command.instance_id.clone(),
            plugin_id: self.plugin_id.clone(),
            editable: self.editable(),
            config_json,
            schema_json,
        })
    }

    fn validation_errors(
        &self,
        config_json: &str,
        current: &Value,
    ) -> Result<Vec<String>, ControlError> {
        let candidate = match serde_json::from_str::<Value>(config_json) {
            Ok(value) => value,
            Err(error) => return Ok(vec![format!("parse candidate JSON failed: {error}")]),
        };
        let schema = self.schema.as_ref().ok_or_else(|| {
            ControlError::new(
                "plugin_config_read_only",
                "plugin configuration has no JSON schema",
            )
        })?;
        let validator = jsonschema::validator_for(schema).map_err(|error| {
            ControlError::new(
                "plugin_config",
                format!(
                    "compile JSON schema for plugin {} failed: {error}",
                    self.plugin_id
                ),
            )
        })?;
        let mut errors = validator
            .iter_errors(&candidate)
            .take(8)
            .map(|error| format!("{}: {error}", error.instance_path()))
            .collect::<Vec<_>>();
        Self::collect_read_only_changes(schema, current, &candidate, "", &mut errors);
        Ok(errors)
    }

    fn collect_read_only_changes(
        schema: &Value,
        current: &Value,
        candidate: &Value,
        path: &str,
        errors: &mut Vec<String>,
    ) {
        if schema.get("readOnly").and_then(Value::as_bool) == Some(true) {
            if current != candidate {
                errors.push(format!(
                    "{} is read-only",
                    if path.is_empty() { "/" } else { path }
                ));
            }
            return;
        }
        let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
            return;
        };
        for (name, property_schema) in properties {
            let child_path = format!("{path}/{}", name.replace('~', "~0").replace('/', "~1"));
            Self::collect_read_only_changes(
                property_schema,
                current.get(name).unwrap_or(&Value::Null),
                candidate.get(name).unwrap_or(&Value::Null),
                &child_path,
                errors,
            );
        }
    }
}
