use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use control_contract::reply::{ControlError, PluginConfigReply, PluginConfigValidationReply};
use plugin_system::PluginManifest;
use sandbox_resource_alert::SandboxResourceAlertConfig;
use serde_json::Value;

static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(1);

pub(super) struct ResourceAlertConfiguration {
    path: PathBuf,
    schema: Value,
    schema_json: String,
    current: SandboxResourceAlertConfig,
}

impl ResourceAlertConfiguration {
    pub(super) fn load(
        path: PathBuf,
        manifest_path: &Path,
        manifest: &PluginManifest,
    ) -> Result<Self, ControlError> {
        if !path.is_absolute() || path.file_name().is_none() {
            return Err(ControlError::new(
                "plugin_config",
                "sandbox resource alert config path must be an absolute file path",
            ));
        }
        let raw = std::fs::read_to_string(&path).map_err(|error| {
            ControlError::new(
                "plugin_config",
                format!("read {} failed: {error}", path.display()),
            )
        })?;
        let current = SandboxResourceAlertConfig::from_json(&raw).map_err(|error| {
            ControlError::new(
                "plugin_config",
                format!("parse {} failed: {error}", path.display()),
            )
        })?;
        let schema_path = Self::schema_path(manifest_path, manifest)?;
        let schema_raw = std::fs::read_to_string(&schema_path).map_err(|error| {
            ControlError::new(
                "plugin_config",
                format!("read {} failed: {error}", schema_path.display()),
            )
        })?;
        let schema = serde_json::from_str::<Value>(&schema_raw).map_err(|error| {
            ControlError::new(
                "plugin_config",
                format!("parse {} failed: {error}", schema_path.display()),
            )
        })?;
        jsonschema::validator_for(&schema).map_err(|error| {
            ControlError::new("plugin_config", format!("compile config schema: {error}"))
        })?;
        let schema_json = serde_json::to_string(&schema)
            .map_err(|error| ControlError::new("plugin_config", error.to_string()))?;
        let configuration = Self {
            path,
            schema,
            schema_json,
            current,
        };
        configuration.require_schema_valid(
            &serde_json::to_value(current)
                .map_err(|error| ControlError::new("plugin_config", error.to_string()))?,
        )?;
        Ok(configuration)
    }

    pub(super) fn current(&self) -> SandboxResourceAlertConfig {
        self.current
    }

    pub(super) fn document(
        &self,
        instance_id: &str,
        plugin_id: &str,
    ) -> Result<PluginConfigReply, ControlError> {
        Ok(PluginConfigReply {
            instance_id: instance_id.to_string(),
            plugin_id: plugin_id.to_string(),
            editable: true,
            config_json: self
                .current
                .to_json()
                .map_err(|error| ControlError::new("plugin_config", error.to_string()))?,
            schema_json: self.schema_json.clone(),
        })
    }

    pub(super) fn validate(
        &self,
        instance_id: &str,
        raw: &str,
    ) -> Result<PluginConfigValidationReply, ControlError> {
        let errors = self.validation_errors(raw)?;
        Ok(PluginConfigValidationReply {
            instance_id: instance_id.to_string(),
            valid: errors.is_empty(),
            errors,
        })
    }

    pub(super) fn prepare(&self, raw: &str) -> Result<SandboxResourceAlertConfig, ControlError> {
        let errors = self.validation_errors(raw)?;
        if !errors.is_empty() {
            return Err(ControlError::new(
                "plugin_config_validation",
                errors.join("; "),
            ));
        }
        SandboxResourceAlertConfig::from_json(raw)
            .map_err(|error| ControlError::new("plugin_config_validation", error.to_string()))
    }

    pub(super) fn persist(&self, config: SandboxResourceAlertConfig) -> Result<(), ControlError> {
        let raw = config
            .to_json()
            .map_err(|error| ControlError::new("plugin_config", error.to_string()))?;
        let parent = self.path.parent().ok_or_else(|| {
            ControlError::new("plugin_config", "config path has no parent directory")
        })?;
        let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let name = self.path.file_name().expect("validated config file name");
        let temporary = self.path.with_file_name(format!(
            ".{}.tmp.{}.{}",
            name.to_string_lossy(),
            std::process::id(),
            sequence
        ));
        let backup = self.path.with_file_name(format!(
            ".{}.backup.{}.{}",
            name.to_string_lossy(),
            std::process::id(),
            sequence
        ));
        let result = self.replace(&temporary, &backup, parent, raw.as_bytes());
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        result
    }

    pub(super) fn commit(&mut self, config: SandboxResourceAlertConfig) {
        self.current = config;
    }

    fn validation_errors(&self, raw: &str) -> Result<Vec<String>, ControlError> {
        let candidate = match serde_json::from_str::<Value>(raw) {
            Ok(candidate) => candidate,
            Err(error) => return Ok(vec![format!("parse candidate JSON failed: {error}")]),
        };
        let validator = jsonschema::validator_for(&self.schema).map_err(|error| {
            ControlError::new("plugin_config", format!("compile config schema: {error}"))
        })?;
        let mut errors = validator
            .iter_errors(&candidate)
            .take(8)
            .map(|error| format!("{}: {error}", error.instance_path()))
            .collect::<Vec<_>>();
        match SandboxResourceAlertConfig::from_json(raw) {
            Ok(config) => {
                let current = serde_json::to_value(self.current)
                    .map_err(|error| ControlError::new("plugin_config", error.to_string()))?;
                let candidate = serde_json::to_value(config)
                    .map_err(|error| ControlError::new("plugin_config", error.to_string()))?;
                Self::collect_read_only_changes(
                    &self.schema,
                    &current,
                    &candidate,
                    "",
                    &mut errors,
                );
            }
            Err(error) if errors.is_empty() => errors.push(error.to_string()),
            Err(_) => {}
        }
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

    fn require_schema_valid(&self, value: &Value) -> Result<(), ControlError> {
        let errors = jsonschema::validator_for(&self.schema)
            .map_err(|error| ControlError::new("plugin_config", error.to_string()))?
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

    fn replace(
        &self,
        temporary: &Path,
        backup: &Path,
        parent: &Path,
        raw: &[u8],
    ) -> Result<(), ControlError> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(temporary)
            .map_err(|error| Self::io_error("create replacement", temporary, error))?;
        file.write_all(raw)
            .map_err(|error| Self::io_error("write replacement", temporary, error))?;
        file.sync_all()
            .map_err(|error| Self::io_error("sync replacement", temporary, error))?;
        std::fs::hard_link(&self.path, backup)
            .map_err(|error| Self::io_error("create rollback link", &self.path, error))?;
        if let Err(error) = Self::sync_directory(parent) {
            let _ = std::fs::remove_file(backup);
            return Err(error);
        }
        if let Err(error) = std::fs::rename(temporary, &self.path) {
            let _ = std::fs::remove_file(backup);
            let _ = Self::sync_directory(parent);
            return Err(Self::io_error("replace", &self.path, error));
        }
        if let Err(error) = Self::sync_directory(parent) {
            std::fs::rename(backup, &self.path).map_err(|restore| {
                ControlError::new(
                    "plugin_config",
                    format!(
                        "{}: {}; restore {} failed: {restore}",
                        error.code,
                        error.message,
                        self.path.display()
                    ),
                )
            })?;
            return match Self::sync_directory(parent) {
                Ok(()) => Err(error),
                Err(restore_sync) => Err(ControlError::new(
                    "plugin_config",
                    format!(
                        "{}: {}; restored {} but {}: {}",
                        error.code,
                        error.message,
                        self.path.display(),
                        restore_sync.code,
                        restore_sync.message
                    ),
                )),
            };
        }
        let _ = std::fs::remove_file(backup);
        let _ = Self::sync_directory(parent);
        Ok(())
    }

    fn sync_directory(path: &Path) -> Result<(), ControlError> {
        File::open(path)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| Self::io_error("sync directory", path, error))
    }

    fn schema_path(
        manifest_path: &Path,
        manifest: &PluginManifest,
    ) -> Result<PathBuf, ControlError> {
        let schema_ref = manifest
            .plugin_config
            .schema_ref
            .as_deref()
            .ok_or_else(|| {
                ControlError::new(
                    "plugin_config",
                    "sandbox resource alert config schema is required",
                )
            })?;
        let raw = PathBuf::from(schema_ref);
        Ok(if raw.is_absolute() {
            raw
        } else {
            manifest_path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(raw)
        })
    }

    fn io_error(operation: &str, path: &Path, error: std::io::Error) -> ControlError {
        ControlError::new(
            "plugin_config",
            format!("{operation} {} failed: {error}", path.display()),
        )
    }
}
