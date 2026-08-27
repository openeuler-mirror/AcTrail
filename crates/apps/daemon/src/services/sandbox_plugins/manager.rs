use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use control_contract::command::PluginLoadCommand;
use control_contract::reply::{ControlError, PluginConfigReply, PluginConfigValidationReply};
use plugin_system::{
    PluginInstanceStatus, PluginLifecycleState, PluginManifest, PluginPurpose, PluginRuntimeKind,
    PluginSandboxObservationKind, SandboxConsumerStatus, SandboxPluginFacade,
    SandboxPluginUnregisterResult,
};
use sandbox_alert_store::SandboxAlertWritePort;
use sandbox_evidence_store::SandboxEvidenceWritePort;
use sandbox_plugin_delivery::{SandboxConsumerId, SandboxObservationKind};
use sandbox_resource_alert::SandboxResourceAlertPlugin;

use super::SandboxPluginRouteSink;
use super::configuration::ResourceAlertConfiguration;

const RESOURCE_ALERT_PLUGIN_ID: &str = "actrail.sandbox-resource-alert";

pub(crate) struct SandboxPluginManager {
    facade: SandboxPluginFacade,
    alert_store: Option<Arc<dyn SandboxAlertWritePort>>,
    instances: BTreeMap<String, SandboxPluginInstance>,
}

struct SandboxPluginInstance {
    plugin_id: String,
    plugin: Arc<SandboxResourceAlertPlugin>,
    configuration: ResourceAlertConfiguration,
    consumer_id: SandboxConsumerId,
    queue_capacity: u32,
}

impl SandboxPluginManager {
    pub(crate) fn new(alert_store: Option<Arc<dyn SandboxAlertWritePort>>) -> Self {
        Self {
            facade: SandboxPluginFacade::new(),
            alert_store,
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
        if self
            .instances
            .values()
            .any(|instance| instance.plugin_id == RESOURCE_ALERT_PLUGIN_ID)
        {
            return Err(ControlError::new(
                "plugin_runtime",
                "sandbox resource alert builtin supports one active instance",
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
        let selectors = kinds
            .iter()
            .copied()
            .map(Self::observation_kind)
            .collect::<Vec<_>>();
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
        let configuration = ResourceAlertConfiguration::load(
            PathBuf::from(config_path),
            &manifest_path,
            &manifest,
        )?;
        let config = configuration.current();
        let alert_store = self.alert_store.as_ref().ok_or_else(|| {
            ControlError::new(
                "sandbox_alerts_disabled",
                "sandbox alert storage must be enabled before loading the resource alert plugin",
            )
        })?;
        let plugin = Arc::new(
            SandboxResourceAlertPlugin::new(config, Arc::clone(alert_store))
                .map_err(|error| ControlError::new("plugin_config", error.to_string()))?,
        );
        let consumer_id = plugin
            .register(&self.facade, selectors, queue_capacity)
            .map_err(|error| {
                ControlError::new(
                    "plugin_runtime",
                    format!("register sandbox plugin: {error:?}"),
                )
            })?;
        self.instances.insert(
            command.instance_id.clone(),
            SandboxPluginInstance {
                plugin_id: manifest.id().to_string(),
                plugin,
                configuration,
                consumer_id,
                queue_capacity,
            },
        );
        self.status(&command.instance_id)
    }

    pub(crate) fn unload(
        &mut self,
        instance_id: &str,
    ) -> Result<PluginInstanceStatus, ControlError> {
        let instance = self.instances.remove(instance_id).ok_or_else(|| {
            ControlError::new(
                "plugin_not_found",
                format!("sandbox plugin instance {instance_id} not found"),
            )
        })?;
        let unregister = self.facade.unregister(instance.consumer_id);
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
        unregister_result?;
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

    pub(crate) fn config(&self, instance_id: &str) -> Result<PluginConfigReply, ControlError> {
        let instance = self.required(instance_id)?;
        instance
            .configuration
            .document(instance_id, &instance.plugin_id)
    }

    pub(crate) fn validate_config(
        &self,
        instance_id: &str,
        config_json: &str,
    ) -> Result<PluginConfigValidationReply, ControlError> {
        let instance = self.required(instance_id)?;
        instance.configuration.validate(instance_id, config_json)
    }

    pub(crate) fn update_config(
        &mut self,
        instance_id: &str,
        config_json: &str,
    ) -> Result<PluginConfigReply, ControlError> {
        let instance = self.required_mut(instance_id)?;
        let config = instance.configuration.prepare(config_json)?;
        instance.configuration.persist(config)?;
        instance.plugin.publish_config(config);
        instance.configuration.commit(config);
        self.config(instance_id)
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

    fn required(&self, instance_id: &str) -> Result<&SandboxPluginInstance, ControlError> {
        self.instances.get(instance_id).ok_or_else(|| {
            ControlError::new(
                "plugin_not_found",
                format!("sandbox plugin instance {instance_id} not found"),
            )
        })
    }

    fn required_mut(
        &mut self,
        instance_id: &str,
    ) -> Result<&mut SandboxPluginInstance, ControlError> {
        self.instances.get_mut(instance_id).ok_or_else(|| {
            ControlError::new(
                "plugin_not_found",
                format!("sandbox plugin instance {instance_id} not found"),
            )
        })
    }

    fn observation_kind(kind: PluginSandboxObservationKind) -> SandboxObservationKind {
        match kind {
            PluginSandboxObservationKind::ProcessIo => SandboxObservationKind::ProcessIo,
            PluginSandboxObservationKind::GuestResource => SandboxObservationKind::GuestResource,
            PluginSandboxObservationKind::OomVictim => SandboxObservationKind::OomVictim,
        }
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
