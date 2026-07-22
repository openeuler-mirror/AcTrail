//! Plugin lifecycle operations for the storage-backed attach service.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use control_contract::command::PluginLoadCommand;
use control_contract::reply::ControlError;
use export_core::ExportPublishReport;
use plugin_system::{
    ControlDecider, PluginCapability, PluginHostGrant, PluginHostGrants, PluginInstanceStatus,
    PluginLifecycleState, PluginManifest, PluginPurpose, PluginRuntimeKind,
};
use recording_runtime::{RecordingError, RecordingWriter};

use crate::services::live::next_diagnostic_id_from_seed;

use super::StorageAttachService;

impl StorageAttachService {
    pub(super) fn plugin_statuses_impl(&self) -> Vec<PluginInstanceStatus> {
        let mut statuses = self.export_runtime.plugin_statuses();
        statuses.extend(self.control_plugins.plugin_statuses());
        statuses.extend(
            self.semantic_actions
                .llm_codec_statuses()
                .into_iter()
                .map(|status| PluginInstanceStatus {
                    instance_id: status.instance_id,
                    plugin_id: status.plugin_id,
                    purpose: PluginPurpose::LlmCodec,
                    runtime: PluginRuntimeKind::Wasm,
                    state: PluginLifecycleState::Active,
                    host_grants: Vec::new(),
                    queue_depth: None,
                    queue_capacity: None,
                    observed_records: 0,
                    dropped_records: 0,
                    hostcall_metrics: Default::default(),
                    last_error: None,
                    warnings: Vec::new(),
                }),
        );
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
        let manifest = toml::from_str::<PluginManifest>(&manifest_raw).map_err(|error| {
            ControlError::new(
                "plugin_manifest",
                format!("parse {} failed: {error}", command.manifest_path),
            )
        })?;
        let manifest_warnings = manifest
            .validate_loadable()
            .map_err(|message| ControlError::new("plugin_manifest", message))?;
        let host_grants = validate_plugin_capability_grants(&manifest, &command.host_grants)?;
        let inspected_config = super::plugin_config::PluginConfigManager::inspect_load(
            &command,
            &manifest_path,
            &manifest,
        )?;
        self.install_plugin_impl(
            command,
            manifest_path,
            manifest,
            manifest_warnings,
            host_grants,
            inspected_config,
        )
    }

    fn install_plugin_impl(
        &mut self,
        command: PluginLoadCommand,
        manifest_path: PathBuf,
        mut manifest: PluginManifest,
        manifest_warnings: Vec<String>,
        host_grants: PluginHostGrants,
        inspected_config: super::plugin_config::InspectedPluginConfig,
    ) -> Result<PluginInstanceStatus, ControlError> {
        let plugin_config_raw = inspected_config.raw.as_deref();
        resolve_manifest_artifact_path(&manifest_path, &mut manifest);
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
        let status = match manifest.role() {
            PluginPurpose::ObservationConsumer => {
                let alert_registered = host_grants.can_write_alerts();
                let alert_host = if alert_registered {
                    Some(self.alert_ingress.register_plugin(
                        &command.instance_id,
                        &manifest_path,
                        &manifest,
                        self.storage.as_mut(),
                    )?)
                } else {
                    None
                };
                let post_trace_host = if manifest.has_post_trace_analyzer() {
                    match self
                        .post_trace_broker
                        .register_plugin(&command.instance_id, &manifest)
                    {
                        Ok(host) => Some(host),
                        Err(error) => {
                            if alert_registered {
                                self.close_and_drain_alert_instance_impl(&command.instance_id)?;
                                self.unregister_alert_instance_impl(&command.instance_id)?;
                            }
                            return Err(error);
                        }
                    }
                } else {
                    None
                };
                let host = post_trace_host
                    .clone()
                    .map(|host| host as Arc<dyn plugin_system::PostTraceHost>);
                let consumer = match export_factory::build_observation_consumer_from_manifest(
                    &command.instance_id,
                    &manifest,
                    plugin_config_raw.as_deref(),
                    host_grants,
                    host,
                    alert_host,
                ) {
                    Ok(consumer) => consumer,
                    Err(error) => {
                        self.post_trace_broker
                            .unregister_plugin(&command.instance_id);
                        if alert_registered {
                            self.close_and_drain_alert_instance_impl(&command.instance_id)?;
                            self.unregister_alert_instance_impl(&command.instance_id)?;
                        }
                        return Err(ControlError::new(error.code, error.message));
                    }
                };
                match self
                    .export_runtime
                    .add_observation_consumer(consumer, manifest_warnings)
                {
                    Ok(status) => Ok(status),
                    Err(error) => {
                        self.post_trace_broker
                            .unregister_plugin(&command.instance_id);
                        if alert_registered {
                            self.close_and_drain_alert_instance_impl(&command.instance_id)?;
                            self.unregister_alert_instance_impl(&command.instance_id)?;
                        }
                        Err(ControlError::new(error.code, error.message))
                    }
                }
            }
            PluginPurpose::ControlDecider => {
                let file_policy_host = self
                    .enforcement
                    .file_policy_host(self.control_plugins.clone());
                let decider = build_control_decider_from_manifest(
                    &command.instance_id,
                    &manifest,
                    plugin_config_raw.as_deref(),
                    host_grants,
                    Some(std::sync::Arc::new(file_policy_host)),
                )?;
                self.control_plugins.add_decider(decider, manifest_warnings)
            }
            PluginPurpose::LlmCodec => {
                let codec = plugin_wasm_runtime::build_wasm_llm_codec_plugin(
                    &command.instance_id,
                    &manifest,
                    plugin_config_raw.as_deref(),
                    host_grants,
                )
                .map_err(|error| ControlError::new(error.code, error.message))?;
                let status = PluginInstanceStatus {
                    instance_id: command.instance_id.clone(),
                    plugin_id: manifest.id().to_string(),
                    purpose: PluginPurpose::LlmCodec,
                    runtime: codec.runtime_kind(),
                    state: PluginLifecycleState::Active,
                    host_grants: codec.host_grants(),
                    queue_depth: None,
                    queue_capacity: None,
                    observed_records: 0,
                    dropped_records: 0,
                    hostcall_metrics: codec
                        .hostcall_metrics_source()
                        .as_ref()
                        .map(|metrics| metrics.snapshot())
                        .unwrap_or_default(),
                    last_error: None,
                    warnings: manifest_warnings,
                };
                self.semantic_actions
                    .register_llm_codec(Arc::new(codec))
                    .map_err(|message| ControlError::new("plugin_runtime", message))?;
                Ok(status)
            }
        }?;
        self.plugin_configs.register(inspected_config);
        Ok(status)
    }

    fn install_updated_plugin_impl(
        &mut self,
        inspected_config: super::plugin_config::InspectedPluginConfig,
    ) -> Result<PluginInstanceStatus, ControlError> {
        let command = inspected_config.load_command();
        let manifest_path = PathBuf::from(&command.manifest_path);
        let manifest_raw = std::fs::read_to_string(&manifest_path).map_err(|error| {
            ControlError::new(
                "plugin_manifest",
                format!("read {} failed: {error}", command.manifest_path),
            )
        })?;
        let manifest = toml::from_str::<PluginManifest>(&manifest_raw).map_err(|error| {
            ControlError::new(
                "plugin_manifest",
                format!("parse {} failed: {error}", command.manifest_path),
            )
        })?;
        let manifest_warnings = manifest
            .validate_loadable()
            .map_err(|message| ControlError::new("plugin_manifest", message))?;
        let host_grants = validate_plugin_capability_grants(&manifest, &command.host_grants)?;
        self.install_plugin_impl(
            command,
            manifest_path,
            manifest,
            manifest_warnings,
            host_grants,
            inspected_config,
        )
    }

    pub(super) fn unload_plugin_impl(
        &mut self,
        instance_id: &str,
    ) -> Result<PluginInstanceStatus, ControlError> {
        let status = self.remove_plugin_runtime_impl(instance_id)?;
        self.plugin_configs.remove(instance_id);
        Ok(status)
    }

    fn remove_plugin_runtime_impl(
        &mut self,
        instance_id: &str,
    ) -> Result<PluginInstanceStatus, ControlError> {
        if let Some(existing) = self
            .export_runtime
            .plugin_statuses()
            .into_iter()
            .find(|status| status.instance_id == instance_id)
        {
            let alert_registered = existing
                .host_grants
                .iter()
                .any(|grant| grant == PluginCapability::AlertWrite.as_str());
            self.drain_post_trace_instance_for_unload_impl(instance_id)?;
            let removal = self
                .export_runtime
                .remove_observation_consumer(instance_id)
                .map_err(|error| ControlError::new(error.code, error.message))?;
            if alert_registered {
                self.close_and_drain_alert_instance_impl(instance_id)?;
                self.unregister_alert_instance_impl(instance_id)?;
            }
            self.post_trace_broker.unregister_plugin(instance_id);
            self.persist_export_drop_report(removal.drop_report)?;
            return Ok(removal.status);
        }
        if self
            .control_plugins
            .plugin_statuses()
            .iter()
            .any(|status| status.instance_id == instance_id)
        {
            self.enforcement.remove_plugin_policy_owner(instance_id)?;
            return self.control_plugins.remove_decider(instance_id);
        }
        if let Some(existing) = self
            .semantic_actions
            .llm_codec_statuses()
            .into_iter()
            .find(|status| status.instance_id == instance_id)
            && self.semantic_actions.unregister_llm_codec(instance_id)
        {
            return Ok(PluginInstanceStatus {
                instance_id: existing.instance_id,
                plugin_id: existing.plugin_id,
                purpose: PluginPurpose::LlmCodec,
                runtime: PluginRuntimeKind::Wasm,
                state: PluginLifecycleState::Stopped,
                host_grants: Vec::new(),
                queue_depth: None,
                queue_capacity: None,
                observed_records: 0,
                dropped_records: 0,
                hostcall_metrics: Default::default(),
                last_error: None,
                warnings: Vec::new(),
            });
        }
        Err(ControlError::new(
            "plugin_not_found",
            format!("plugin instance {instance_id} not found"),
        ))
    }

    pub(super) fn plugin_config_impl(
        &self,
        instance_id: &str,
    ) -> Result<control_contract::reply::PluginConfigReply, ControlError> {
        if !self.plugin_configs.runtime_managed(instance_id)? {
            return self.plugin_configs.document(instance_id);
        }
        let current = self
            .control_plugins
            .runtime_config(instance_id)
            .map_err(|error| ControlError::new(error.code, error.message))?;
        self.plugin_configs
            .runtime_document(instance_id, &current.config_json)
    }

    pub(super) fn validate_plugin_config_impl(
        &self,
        instance_id: &str,
        config_json: &str,
    ) -> Result<control_contract::reply::PluginConfigValidationReply, ControlError> {
        if !self.plugin_configs.runtime_managed(instance_id)? {
            return self.plugin_configs.validate(instance_id, config_json);
        }
        let current = self
            .control_plugins
            .runtime_config(instance_id)
            .map_err(|error| ControlError::new(error.code, error.message))?;
        let mut validation =
            self.plugin_configs
                .validate_runtime(instance_id, &current.config_json, config_json)?;
        if validation.valid {
            validation.errors = self
                .control_plugins
                .validate_runtime_config(instance_id, config_json)
                .map_err(|error| ControlError::new(error.code, error.message))?;
            validation.valid = validation.errors.is_empty();
        }
        Ok(validation)
    }

    pub(super) fn update_plugin_config_impl(
        &mut self,
        instance_id: &str,
        config_json: &str,
    ) -> Result<control_contract::reply::PluginConfigReply, ControlError> {
        if self.plugin_configs.runtime_managed(instance_id)? {
            let current = self
                .control_plugins
                .runtime_config(instance_id)
                .map_err(|error| ControlError::new(error.code, error.message))?;
            let update = self.plugin_configs.prepare_runtime_update(
                instance_id,
                &current.config_json,
                config_json,
            )?;
            let canonical_json = update.raw.as_deref().ok_or_else(|| {
                ControlError::new(
                    "plugin_config",
                    "runtime config update has no JSON document",
                )
            })?;
            let errors = self
                .control_plugins
                .validate_runtime_config(instance_id, canonical_json)
                .map_err(|error| ControlError::new(error.code, error.message))?;
            if !errors.is_empty() {
                return Err(ControlError::new(
                    "plugin_config_validation",
                    errors.join("; "),
                ));
            }
            self.control_plugins
                .submit_runtime_config(instance_id, canonical_json)
                .map_err(|error| ControlError::new(error.code, error.message))?;
            let current = self
                .control_plugins
                .runtime_config(instance_id)
                .map_err(|error| ControlError::new(error.code, error.message))?;
            self.plugin_configs
                .commit_runtime_config(instance_id, &current.config_json)?;
            return self
                .plugin_configs
                .runtime_document(instance_id, &current.config_json);
        }
        let update = self
            .plugin_configs
            .prepare_update(instance_id, config_json)?;
        self.remove_plugin_runtime_impl(instance_id)?;
        self.plugin_configs.remove(instance_id);
        self.install_updated_plugin_impl(update)?;
        self.plugin_configs.document(instance_id)
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
            PluginCapability::TraceAnalysisRead if !host_grants.can_read_trace_analysis() => {
                ungranted.push(capability.as_str());
            }
            PluginCapability::TraceAnalysisRead => {}
            PluginCapability::TraceFileStateRead if !host_grants.can_read_trace_file_state() => {
                ungranted.push(capability.as_str());
            }
            PluginCapability::TraceFileStateRead => {}
            PluginCapability::AlertWrite if !host_grants.can_write_alerts() => {
                ungranted.push(capability.as_str());
            }
            PluginCapability::AlertWrite => {}
            PluginCapability::FileAccessCurrentMatchGet
                if !host_grants.can_get_current_file_access_match() =>
            {
                ungranted.push(capability.as_str());
            }
            PluginCapability::FileAccessCurrentMatchGet => {}
            PluginCapability::FileAccessCurrentContextQuery
                if !host_grants.can_query_current_file_access_context() =>
            {
                ungranted.push(capability.as_str());
            }
            PluginCapability::FileAccessCurrentContextQuery => {}
            PluginCapability::FilePolicyRulesRead if !host_grants.can_read_file_policy_rules() => {
                ungranted.push(capability.as_str());
            }
            PluginCapability::FilePolicyRulesRead => {}
            PluginCapability::FilePolicyRulesMatchDryRun
                if !host_grants.can_match_dry_run_file_policy_rules() =>
            {
                ungranted.push(capability.as_str());
            }
            PluginCapability::FilePolicyRulesMatchDryRun => {}
            PluginCapability::FilePolicyRulesValidate
                if !host_grants.can_validate_file_policy_rules() =>
            {
                ungranted.push(capability.as_str());
            }
            PluginCapability::FilePolicyRulesValidate => {}
            PluginCapability::FilePolicyRulesApply
                if !host_grants.can_apply_file_policy_rules() =>
            {
                ungranted.push(capability.as_str());
            }
            PluginCapability::FilePolicyRulesApply => {}
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

fn build_control_decider_from_manifest(
    instance_id: &str,
    manifest: &PluginManifest,
    plugin_config: Option<&str>,
    host_grants: PluginHostGrants,
    file_policy_host: Option<std::sync::Arc<dyn plugin_system::FilePolicyHost>>,
) -> Result<Box<dyn ControlDecider>, ControlError> {
    match manifest.runtime_kind() {
        PluginRuntimeKind::Wasm => {
            let decider = plugin_wasm_runtime::build_wasm_control_decider(
                instance_id,
                manifest,
                plugin_config,
                host_grants,
                file_policy_host,
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
