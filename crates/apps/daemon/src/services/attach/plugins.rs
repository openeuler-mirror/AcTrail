//! Plugin lifecycle operations for the storage-backed attach service.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use control_contract::command::PluginLoadCommand;
use control_contract::reply::ControlError;
use export_core::ExportPublishReport;
use plugin_system::{
    ControlDecider, PluginCapability, PluginHostGrant, PluginHostGrants, PluginInstanceStatus,
    PluginManifest, PluginPurpose, PluginRuntimeKind,
};
use recording_runtime::{RecordingError, RecordingWriter};

use crate::services::live::next_diagnostic_id_from_seed;

use super::StorageAttachService;

impl StorageAttachService {
    pub(super) fn plugin_statuses_impl(&self) -> Vec<PluginInstanceStatus> {
        let mut statuses = self.export_runtime.plugin_statuses();
        statuses.extend(self.control_plugins.plugin_statuses());
        statuses
    }

    pub(super) fn load_plugin_impl(
        &mut self,
        command: PluginLoadCommand,
    ) -> Result<PluginInstanceStatus, ControlError> {
        let manifest_path = PathBuf::from(&command.manifest_path);
        let manifest_raw = std::fs::read_to_string(&manifest_path).map_err(|error| {
            ControlError::new(
                "plugin_manifest",
                format!("read {} failed: {error}", command.manifest_path),
            )
        })?;
        let mut manifest = toml::from_str::<PluginManifest>(&manifest_raw).map_err(|error| {
            ControlError::new(
                "plugin_manifest",
                format!("parse {} failed: {error}", command.manifest_path),
            )
        })?;
        let manifest_warnings = manifest
            .validate_loadable()
            .map_err(|message| ControlError::new("plugin_manifest", message))?;
        let host_grants = validate_plugin_capability_grants(&manifest, &command.host_grants)?;
        resolve_manifest_artifact_path(&manifest_path, &mut manifest);
        if manifest.plugin_config.required && command.plugin_config_path.is_none() {
            return Err(ControlError::new(
                "plugin_config",
                format!("plugin {} requires --plugin-config", manifest.id()),
            ));
        }
        let plugin_config_raw = match command.plugin_config_path.as_ref() {
            Some(path) => Some(std::fs::read_to_string(path).map_err(|error| {
                ControlError::new("plugin_config", format!("read {path} failed: {error}"))
            })?),
            None => None,
        };
        validate_plugin_config(&manifest_path, &manifest, plugin_config_raw.as_deref())?;
        if self
            .plugin_statuses_impl()
            .iter()
            .any(|status| status.instance_id == command.instance_id)
        {
            return Err(ControlError::new(
                "plugin_runtime",
                format!("plugin instance {} already exists", command.instance_id),
            ));
        }
        match manifest.role() {
            PluginPurpose::ObservationConsumer => {
                let consumer = export_factory::build_observation_consumer_from_manifest(
                    &command.instance_id,
                    &manifest,
                    plugin_config_raw.as_deref(),
                    host_grants,
                )
                .map_err(|error| ControlError::new(error.code, error.message))?;
                self.export_runtime
                    .add_observation_consumer(consumer, manifest_warnings)
                    .map_err(|error| ControlError::new(error.code, error.message))
            }
            PluginPurpose::ControlDecider => {
                let decider = build_control_decider_from_manifest(
                    &command.instance_id,
                    &manifest,
                    plugin_config_raw.as_deref(),
                    host_grants,
                )?;
                self.control_plugins.add_decider(decider, manifest_warnings)
            }
        }
    }

    pub(super) fn unload_plugin_impl(
        &mut self,
        instance_id: &str,
    ) -> Result<PluginInstanceStatus, ControlError> {
        if self
            .export_runtime
            .plugin_statuses()
            .iter()
            .any(|status| status.instance_id == instance_id)
        {
            let removal = self
                .export_runtime
                .remove_observation_consumer(instance_id)
                .map_err(|error| ControlError::new(error.code, error.message))?;
            self.persist_export_drop_report(removal.drop_report)?;
            return Ok(removal.status);
        }
        if self
            .control_plugins
            .plugin_statuses()
            .iter()
            .any(|status| status.instance_id == instance_id)
        {
            return self.control_plugins.remove_decider(instance_id);
        }
        Err(ControlError::new(
            "plugin_not_found",
            format!("plugin instance {instance_id} not found"),
        ))
    }

    fn persist_export_drop_report(
        &mut self,
        report: ExportPublishReport,
    ) -> Result<(), ControlError> {
        let next_diagnostic_id = &mut self.next_diagnostic_id;
        RecordingWriter::new(self.storage.as_mut())
            .persist_export_drop_report(report, SystemTime::now(), || {
                next_diagnostic_id_from_seed(next_diagnostic_id).map_err(control_error_to_recording)
            })
            .map_err(recording_error_to_control)
    }
}

fn recording_error_to_control(error: RecordingError) -> ControlError {
    ControlError::new(error.stage, error.message)
}

fn control_error_to_recording(error: ControlError) -> RecordingError {
    RecordingError::new(error.code, error.message)
}

fn validate_plugin_capability_grants(
    manifest: &PluginManifest,
    raw_grants: &[String],
) -> Result<PluginHostGrants, ControlError> {
    let host_grants = PluginHostGrants::parse(raw_grants)
        .map_err(|message| ControlError::new("plugin_capability", message))?;
    for raw_grant in raw_grants {
        let grant = PluginHostGrant::parse(raw_grant)
            .map_err(|message| ControlError::new("plugin_capability", message))?;
        let grant_capability = grant.capability();
        if !manifest
            .capabilities()
            .iter()
            .any(|capability| capability == &grant_capability)
        {
            return Err(ControlError::new(
                "plugin_capability",
                format!(
                    "plugin {} was granted {} but did not request {}",
                    manifest.id(),
                    raw_grant,
                    grant_capability.as_str()
                ),
            ));
        }
    }
    let mut ungranted = Vec::new();
    for capability in manifest.capabilities() {
        match capability {
            PluginCapability::PayloadRead if !host_grants.can_read_payload() => {
                ungranted.push(capability.as_str());
            }
            PluginCapability::PayloadRead => {}
            PluginCapability::EnvRead if host_grants.env_read_names().next().is_none() => {
                ungranted.push(capability.as_str());
            }
            PluginCapability::EnvRead => {}
            PluginCapability::ContextQuery if !host_grants.can_query_context() => {
                ungranted.push(capability.as_str());
            }
            PluginCapability::ContextQuery => {}
            PluginCapability::FilePolicyRead if !host_grants.can_read_file_policy() => {
                ungranted.push(capability.as_str());
            }
            PluginCapability::FilePolicyRead => {}
            PluginCapability::FilePolicyWrite if !host_grants.can_write_file_policy() => {
                ungranted.push(capability.as_str());
            }
            PluginCapability::FilePolicyWrite => {}
            other => ungranted.push(other.as_str()),
        }
    }
    if !ungranted.is_empty() {
        return Err(ControlError::new(
            "plugin_capability",
            format!(
                "plugin {} requested ungranted host capabilities: {}",
                manifest.id(),
                ungranted.join(", ")
            ),
        ));
    }
    Ok(host_grants)
}

fn validate_plugin_config(
    manifest_path: &Path,
    manifest: &PluginManifest,
    plugin_config: Option<&str>,
) -> Result<(), ControlError> {
    let Some(plugin_config) = plugin_config else {
        return Ok(());
    };
    let config = match manifest.plugin_config.format.as_str() {
        "toml" => toml::from_str::<toml::Value>(plugin_config).map_err(|error| {
            ControlError::new(
                "plugin_config",
                format!(
                    "parse TOML config for plugin {} failed: {error}",
                    manifest.id()
                ),
            )
        })?,
        other => {
            return Err(ControlError::new(
                "plugin_config",
                format!(
                    "unsupported format {other} for plugin {}; supported format: toml",
                    manifest.id()
                ),
            ));
        }
    };
    validate_plugin_config_schema(manifest_path, manifest, config)
}

fn validate_plugin_config_schema(
    manifest_path: &Path,
    manifest: &PluginManifest,
    config: toml::Value,
) -> Result<(), ControlError> {
    let Some(schema_ref) = manifest.plugin_config.schema_ref.as_deref() else {
        return Ok(());
    };
    if schema_ref.trim().is_empty() {
        return Err(ControlError::new(
            "plugin_config",
            format!("schema_ref for plugin {} must not be empty", manifest.id()),
        ));
    }
    let schema_path = resolve_schema_path(manifest_path, schema_ref);
    let schema_raw = std::fs::read_to_string(&schema_path).map_err(|error| {
        ControlError::new(
            "plugin_config",
            format!(
                "read schema_ref {} for plugin {} failed: {error}",
                schema_path.display(),
                manifest.id()
            ),
        )
    })?;
    let schema = serde_json::from_str::<serde_json::Value>(&schema_raw).map_err(|error| {
        ControlError::new(
            "plugin_config",
            format!(
                "parse JSON schema_ref {} for plugin {} failed: {error}",
                schema_path.display(),
                manifest.id()
            ),
        )
    })?;
    let config_json = serde_json::to_value(config).map_err(|error| {
        ControlError::new(
            "plugin_config",
            format!(
                "convert TOML config for plugin {} failed: {error}",
                manifest.id()
            ),
        )
    })?;
    let validator = jsonschema::validator_for(&schema).map_err(|error| {
        ControlError::new(
            "plugin_config",
            format!(
                "compile schema_ref {} for plugin {} failed: {error}",
                schema_path.display(),
                manifest.id()
            ),
        )
    })?;
    let errors = validator.iter_errors(&config_json).collect::<Vec<_>>();
    if errors.is_empty() {
        return Ok(());
    }
    let details = errors
        .iter()
        .take(3)
        .map(|error| format!("{}: {error}", error.instance_path()))
        .collect::<Vec<_>>()
        .join("; ");
    Err(ControlError::new(
        "plugin_config",
        format!(
            "schema validation for plugin {} failed against schema_ref {}: {}",
            manifest.id(),
            schema_path.display(),
            details
        ),
    ))
}

fn resolve_schema_path(manifest_path: &Path, schema_ref: &str) -> PathBuf {
    let raw = PathBuf::from(schema_ref);
    if raw.is_absolute() {
        return raw;
    }
    manifest_path
        .parent()
        .map(|parent| parent.join(&raw))
        .unwrap_or(raw)
}

fn build_control_decider_from_manifest(
    instance_id: &str,
    manifest: &PluginManifest,
    plugin_config: Option<&str>,
    host_grants: PluginHostGrants,
) -> Result<Box<dyn ControlDecider>, ControlError> {
    match manifest.runtime_kind() {
        PluginRuntimeKind::Wasm => {
            let decider = plugin_wasm_runtime::build_wasm_control_decider(
                instance_id,
                manifest,
                plugin_config,
                host_grants,
            )
            .map_err(|error| ControlError::new(error.code, error.message))?;
            Ok(Box::new(decider))
        }
        PluginRuntimeKind::Builtin => Err(ControlError::new(
            "plugin_factory",
            "no builtin control-decider plugins are enabled",
        )),
        PluginRuntimeKind::NativeDylib => Err(ControlError::new(
            "plugin_factory",
            "native dynamic plugins are not enabled",
        )),
    }
}

fn resolve_manifest_artifact_path(manifest_path: &Path, manifest: &mut PluginManifest) {
    let Some(wasm) = manifest.selected_wasm_mut() else {
        return;
    };
    let Some(artifact_path) = wasm.artifact_path.as_mut() else {
        return;
    };
    let raw = PathBuf::from(artifact_path.as_str());
    if raw.is_absolute() {
        return;
    }
    if let Some(parent) = manifest_path.parent() {
        *artifact_path = parent.join(raw).display().to_string();
    }
}
