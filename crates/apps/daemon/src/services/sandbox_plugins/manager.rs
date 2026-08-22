use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use control_contract::command::PluginLoadCommand;
use control_contract::reply::ControlError;
use plugin_system::{
    PluginInstanceStatus, PluginLifecycleState, PluginManifest, PluginPurpose, PluginRuntimeKind,
    PluginSandboxObservationKind, SandboxConsumerStatus, SandboxPluginFacade,
    SandboxPluginUnregisterResult,
};
use sandbox_evidence_store::SandboxEvidenceWritePort;
use sandbox_plugin_delivery::SandboxConsumerId;
use sandbox_resource_alert::{SandboxResourceAlertConfig, SandboxResourceAlertPlugin};
use serde::Deserialize;

use super::SandboxPluginRouteSink;
use super::alert_writer::SandboxAlertWriter;

const RESOURCE_ALERT_PLUGIN_ID: &str = "actrail.sandbox-resource-alert";

pub(crate) struct SandboxPluginManager {
    facade: SandboxPluginFacade,
    instances: BTreeMap<String, SandboxPluginInstance>,
}

struct SandboxPluginInstance {
    plugin_id: String,
    consumer_id: SandboxConsumerId,
    queue_capacity: u32,
    writer: SandboxAlertWriter,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResourceAlertConfigDocument {
    memory_available_threshold_bytes: u64,
    read_interval_threshold_bytes: u64,
    write_interval_threshold_bytes: u64,
    source_state_capacity: u32,
    alert_output_path: PathBuf,
    alert_queue_capacity: usize,
    alert_flush_interval_ms: u64,
    alert_writer_thread_stack_bytes: usize,
}

impl SandboxPluginManager {
    pub(crate) fn new() -> Self {
        Self {
            facade: SandboxPluginFacade::new(),
            instances: BTreeMap::new(),
        }
    }

    pub(crate) fn route_sink(
        &self,
        archive: Arc<dyn SandboxEvidenceWritePort>,
    ) -> SandboxPluginRouteSink {
        SandboxPluginRouteSink::new(self.facade.matcher(), self.facade.publisher(), archive)
    }

    pub(crate) fn is_sandbox_manifest(path: &Path) -> Result<bool, ControlError> {
        Self::read_manifest(path)
            .map(|manifest| manifest.role() == PluginPurpose::SandboxObservationConsumer)
    }

    pub(crate) fn load(
        &mut self,
        command: PluginLoadCommand,
    ) -> Result<PluginInstanceStatus, ControlError> {
        if self.instances.contains_key(&command.instance_id) {
            return Err(ControlError::new(
                "plugin_runtime",
                format!("plugin instance {} already exists", command.instance_id),
            ));
        }
        if !command.host_grants.is_empty() {
            return Err(ControlError::new(
                "plugin_grant",
                "sandbox resource alert builtin does not accept host grants",
            ));
        }
        let manifest_path = PathBuf::from(&command.manifest_path);
        let manifest = Self::read_manifest(&manifest_path)?;
        if manifest.role() != PluginPurpose::SandboxObservationConsumer
            || manifest.runtime_kind() != PluginRuntimeKind::Builtin
            || manifest.id() != RESOURCE_ALERT_PLUGIN_ID
        {
            return Err(ControlError::new(
                "plugin_manifest",
                "unsupported sandbox observation plugin manifest",
            ));
        }
        let kinds = manifest.sandbox_observation_kinds();
        if !kinds.contains(&PluginSandboxObservationKind::ProcessIo)
            || !kinds.contains(&PluginSandboxObservationKind::GuestResource)
        {
            return Err(ControlError::new(
                "plugin_manifest",
                "sandbox resource alert plugin requires process-io and guest-resource subscriptions",
            ));
        }
        let queue_capacity = manifest
            .sandbox_observation_queue_capacity()
            .ok_or_else(|| {
                ControlError::new(
                    "plugin_manifest",
                    "sandbox plugin queue_capacity must be configured",
                )
            })?;
        let config_path = command.plugin_config_path.as_deref().ok_or_else(|| {
            ControlError::new(
                "plugin_config",
                "sandbox resource alert plugin requires a config file",
            )
        })?;
        let config = Self::read_config(Path::new(config_path))?;
        let mut writer = SandboxAlertWriter::start(
            &config.alert_output_path,
            config.alert_queue_capacity,
            Duration::from_millis(config.alert_flush_interval_ms),
            config.alert_writer_thread_stack_bytes,
        )
        .map_err(|error| ControlError::new("sandbox_alert_writer", error.to_string()))?;
        let plugin = Arc::new(
            SandboxResourceAlertPlugin::new(
                SandboxResourceAlertConfig {
                    memory_available_threshold_bytes: config.memory_available_threshold_bytes,
                    read_interval_threshold_bytes: config.read_interval_threshold_bytes,
                    write_interval_threshold_bytes: config.write_interval_threshold_bytes,
                    source_state_capacity: config.source_state_capacity,
                },
                writer.sink(),
            )
            .map_err(|error| ControlError::new("plugin_config", error.to_string()))?,
        );
        let consumer_id = match plugin.register(&self.facade, queue_capacity) {
            Ok(consumer_id) => consumer_id,
            Err(error) => {
                let _ = writer.shutdown();
                return Err(ControlError::new(
                    "plugin_runtime",
                    format!("register sandbox plugin: {error:?}"),
                ));
            }
        };
        self.instances.insert(
            command.instance_id.clone(),
            SandboxPluginInstance {
                plugin_id: manifest.id().to_string(),
                consumer_id,
                queue_capacity,
                writer,
            },
        );
        self.status(&command.instance_id)
    }

    pub(crate) fn unload(
        &mut self,
        instance_id: &str,
    ) -> Result<PluginInstanceStatus, ControlError> {
        let mut instance = self.instances.remove(instance_id).ok_or_else(|| {
            ControlError::new(
                "plugin_not_found",
                format!("sandbox plugin instance {instance_id} not found"),
            )
        })?;
        let unregister = self.facade.unregister(instance.consumer_id);
        let writer_result = instance
            .writer
            .shutdown()
            .map_err(|error| ControlError::new("sandbox_alert_writer", error.to_string()));
        let unregister_result = match unregister {
            SandboxPluginUnregisterResult::Unregistered { .. } => Ok(()),
            SandboxPluginUnregisterResult::NotFound { .. } => Err(ControlError::new(
                "plugin_runtime",
                format!("sandbox consumer for instance {instance_id} was not registered"),
            )),
            SandboxPluginUnregisterResult::RegistryGenerationExhausted => Err(ControlError::new(
                "plugin_runtime",
                "sandbox plugin registry generation is exhausted",
            )),
            SandboxPluginUnregisterResult::RegistryUnavailable => Err(ControlError::new(
                "plugin_runtime",
                "sandbox plugin registry is unavailable",
            )),
            SandboxPluginUnregisterResult::WorkerPanicked { .. } => Err(ControlError::new(
                "plugin_runtime",
                format!("sandbox consumer for instance {instance_id} panicked during shutdown"),
            )),
        };
        unregister_result.and(writer_result)?;
        Ok(Self::instance_status(
            instance_id,
            &instance,
            PluginLifecycleState::Stopped,
            None,
        ))
    }

    pub(crate) fn contains(&self, instance_id: &str) -> bool {
        self.instances.contains_key(instance_id)
    }

    pub(crate) fn statuses(&self) -> Vec<PluginInstanceStatus> {
        let consumers = self
            .facade
            .consumer_statuses()
            .into_iter()
            .map(|status| (status.consumer_id, status))
            .collect::<BTreeMap<_, _>>();
        self.instances
            .iter()
            .map(|(instance_id, instance)| {
                let consumer = consumers.get(&instance.consumer_id).cloned();
                let state = match &consumer {
                    Some(status) if !status.closed => PluginLifecycleState::Active,
                    _ => PluginLifecycleState::Failed,
                };
                Self::instance_status(instance_id, instance, state, consumer)
            })
            .collect()
    }

    pub(crate) fn status(&self, instance_id: &str) -> Result<PluginInstanceStatus, ControlError> {
        self.statuses()
            .into_iter()
            .find(|status| status.instance_id == instance_id)
            .ok_or_else(|| {
                ControlError::new(
                    "plugin_not_found",
                    format!("sandbox plugin instance {instance_id} not found"),
                )
            })
    }

    pub(crate) fn shutdown(&mut self) -> Result<(), ControlError> {
        let ids = self.instances.keys().cloned().collect::<Vec<_>>();
        let mut failures = Vec::new();
        for id in ids {
            if let Err(error) = self.unload(&id) {
                failures.push(format!("{}: {}", error.code, error.message));
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(ControlError::new(
                "sandbox_plugin_shutdown",
                failures.join("; "),
            ))
        }
    }

    fn read_manifest(path: &Path) -> Result<PluginManifest, ControlError> {
        let raw = std::fs::read_to_string(path).map_err(|error| {
            ControlError::new(
                "plugin_manifest",
                format!("read {} failed: {error}", path.display()),
            )
        })?;
        let manifest = toml::from_str::<PluginManifest>(&raw).map_err(|error| {
            ControlError::new(
                "plugin_manifest",
                format!("parse {} failed: {error}", path.display()),
            )
        })?;
        manifest
            .validate_loadable()
            .map_err(|error| ControlError::new("plugin_manifest", error))?;
        Ok(manifest)
    }

    fn read_config(path: &Path) -> Result<ResourceAlertConfigDocument, ControlError> {
        let raw = std::fs::read_to_string(path).map_err(|error| {
            ControlError::new(
                "plugin_config",
                format!("read {} failed: {error}", path.display()),
            )
        })?;
        serde_json::from_str(&raw).map_err(|error| {
            ControlError::new(
                "plugin_config",
                format!("parse {} failed: {error}", path.display()),
            )
        })
    }

    fn instance_status(
        instance_id: &str,
        instance: &SandboxPluginInstance,
        state: PluginLifecycleState,
        consumer: Option<SandboxConsumerStatus>,
    ) -> PluginInstanceStatus {
        PluginInstanceStatus {
            instance_id: instance_id.to_string(),
            plugin_id: instance.plugin_id.clone(),
            purpose: PluginPurpose::SandboxObservationConsumer,
            runtime: PluginRuntimeKind::Builtin,
            state,
            host_grants: Vec::new(),
            queue_depth: consumer.as_ref().map(|value| value.queue_depth),
            queue_capacity: Some(instance.queue_capacity),
            observed_records: consumer
                .as_ref()
                .map(|value| value.observed_records)
                .unwrap_or(0),
            dropped_records: consumer
                .as_ref()
                .map(|value| value.dropped_records)
                .unwrap_or(0),
            hostcall_metrics: Default::default(),
            operational_metrics: Default::default(),
            last_error: consumer.and_then(|value| value.last_error),
            warnings: Vec::new(),
        }
    }
}

impl Default for SandboxPluginManager {
    fn default() -> Self {
        Self::new()
    }
}
