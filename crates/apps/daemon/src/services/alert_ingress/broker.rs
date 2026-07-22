use std::collections::BTreeMap;
use std::os::fd::RawFd;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, sync_channel};
use std::time::Duration;

use alert_contract::AlertDefinition;
use config_core::daemon::PluginAlertRuntimeConfig;
use control_contract::reply::ControlError;
use model_core::ids::TraceId;
use model_core::trace::TraceAlertToken;
use plugin_system::{AlertHost, PluginManifest};
use storage_core::StorageBackend;

use super::protocol::{
    AlertAdmission, AlertHostClient, AlertRequest, EventSignal, RegisteredOutput,
};
use super::system::{self, DAEMON_ENFORCEMENT_INSTANCE_ID, FileAccessBoundaryAlert};

pub(crate) struct AlertIngress {
    request_sender: SyncSender<AlertRequest>,
    request_receiver: Receiver<AlertRequest>,
    signal: Arc<EventSignal>,
    registrations: BTreeMap<String, Arc<AlertAdmission>>,
    daemon_alert_host: AlertHostClient,
    writes_per_cycle: usize,
    drain_timeout: Duration,
}

pub(super) struct AlertIngressIssue {
    pub(super) trace_id: TraceId,
    pub(super) instance_id: String,
    pub(super) code: String,
    pub(super) message: String,
}

impl AlertIngress {
    pub(crate) fn new(
        config: PluginAlertRuntimeConfig,
        storage: &mut dyn StorageBackend,
    ) -> Result<Self, ControlError> {
        let queue_capacity = usize::try_from(config.queue_capacity).map_err(|error| {
            ControlError::new(
                "alert_ingress_config",
                format!("queue capacity overflow: {error}"),
            )
        })?;
        let writes_per_cycle = usize::try_from(config.writes_per_cycle).map_err(|error| {
            ControlError::new(
                "alert_ingress_config",
                format!("writes per cycle overflow: {error}"),
            )
        })?;
        let (request_sender, request_receiver) = sync_channel(queue_capacity);
        let signal = Arc::new(EventSignal::new()?);
        let daemon_admission = system::register(storage)?;
        let daemon_alert_host = AlertHostClient::new(
            Arc::clone(&daemon_admission),
            request_sender.clone(),
            Arc::clone(&signal),
        );
        Ok(Self {
            request_sender,
            request_receiver,
            signal,
            registrations: BTreeMap::from([(
                DAEMON_ENFORCEMENT_INSTANCE_ID.to_string(),
                daemon_admission,
            )]),
            daemon_alert_host,
            writes_per_cycle,
            drain_timeout: Duration::from_millis(config.drain_timeout_ms),
        })
    }

    pub(crate) fn event_poll_fd(&self) -> RawFd {
        self.signal.as_raw_fd()
    }

    pub(crate) fn submit_file_access_boundary_alert(
        &self,
        trace_id: TraceId,
        alert_token: TraceAlertToken,
        alert: FileAccessBoundaryAlert,
    ) -> Result<(), ControlError> {
        self.daemon_alert_host
            .submit_file_access_boundary_alert(trace_id, alert_token, alert)
            .map_err(|error| ControlError::new(error.code, error.message))
    }

    pub(crate) fn register_plugin(
        &mut self,
        instance_id: &str,
        manifest_path: &Path,
        manifest: &PluginManifest,
        storage: &mut dyn StorageBackend,
    ) -> Result<Arc<dyn AlertHost>, ControlError> {
        if self.registrations.contains_key(instance_id) {
            return Err(ControlError::new(
                "alert_ingress_registration",
                format!("plugin instance {instance_id} is already registered"),
            ));
        }
        let mut outputs = BTreeMap::new();
        for (definition_key, declaration) in manifest.alert_outputs() {
            let schema_path = resolve_schema_path(manifest_path, &declaration.payload_schema_ref);
            let raw = std::fs::read_to_string(&schema_path).map_err(|error| {
                ControlError::new(
                    "alert_payload_schema",
                    format!("read {} failed: {error}", schema_path.display()),
                )
            })?;
            let schema = serde_json::from_str::<serde_json::Value>(&raw).map_err(|error| {
                ControlError::new(
                    "alert_payload_schema",
                    format!("parse {} failed: {error}", schema_path.display()),
                )
            })?;
            let schema_id = schema.get("$id").and_then(serde_json::Value::as_str);
            if schema_id != Some(declaration.payload_schema_id.as_str()) {
                return Err(ControlError::new(
                    "alert_payload_schema",
                    format!(
                        "schema {} must declare $id = {:?}",
                        schema_path.display(),
                        declaration.payload_schema_id
                    ),
                ));
            }
            let validator = jsonschema::validator_for(&schema).map_err(|error| {
                ControlError::new(
                    "alert_payload_schema",
                    format!("compile {} failed: {error}", schema_path.display()),
                )
            })?;
            storage
                .register_alert_definition(&AlertDefinition {
                    producer_plugin_id: manifest.id().to_string(),
                    definition_key: definition_key.to_string(),
                    kind: declaration.kind.clone(),
                    title: declaration.title.clone(),
                    severity: declaration.severity,
                    payload_schema_id: declaration.payload_schema_id.clone(),
                })
                .map_err(alert_control_error)?;
            outputs.insert(definition_key.to_string(), RegisteredOutput::new(validator));
        }
        let admission = Arc::new(AlertAdmission::new(
            instance_id.to_string(),
            manifest.id().to_string(),
            outputs,
        ));
        self.registrations
            .insert(instance_id.to_string(), Arc::clone(&admission));
        Ok(Arc::new(AlertHostClient::new(
            admission,
            self.request_sender.clone(),
            Arc::clone(&self.signal),
        )))
    }

    pub(crate) fn close_instance(&self, instance_id: &str) -> Result<(), ControlError> {
        self.registration(instance_id)?.close()
    }

    pub(crate) fn close_all(&self) -> Result<(), ControlError> {
        for admission in self.registrations.values() {
            admission.close()?;
        }
        Ok(())
    }

    pub(crate) fn unregister_plugin(&mut self, instance_id: &str) -> Result<(), ControlError> {
        let admission = self.registration(instance_id)?;
        if admission.has_outstanding_writes()? {
            return Err(ControlError::new(
                "alert_ingress_registration",
                format!("plugin instance {instance_id} still has outstanding alert writes"),
            ));
        }
        self.registrations.remove(instance_id);
        Ok(())
    }

    pub(crate) fn has_outstanding_writes(&self) -> Result<bool, ControlError> {
        for admission in self.registrations.values() {
            if admission.has_outstanding_writes()? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub(crate) fn has_outstanding_writes_for(
        &self,
        instance_id: &str,
    ) -> Result<bool, ControlError> {
        self.registration(instance_id)?.has_outstanding_writes()
    }

    pub(crate) fn drain_timeout(&self) -> Duration {
        self.drain_timeout
    }

    pub(super) fn drain_requests(
        &mut self,
        storage: &mut dyn StorageBackend,
    ) -> Result<Vec<AlertIngressIssue>, ControlError> {
        self.signal.drain()?;
        let mut issues = Vec::new();
        let mut processed = 0_usize;
        while processed < self.writes_per_cycle {
            let request = match self.request_receiver.try_recv() {
                Ok(request) => request,
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            };
            let draft = match request.draft.into_draft() {
                Ok(draft) => draft,
                Err(error) => {
                    request.admission.complete()?;
                    issues.push(AlertIngressIssue {
                        trace_id: request.trace_id,
                        instance_id: request.admission.instance_id.clone(),
                        code: error.code,
                        message: error.message,
                    });
                    processed += 1;
                    continue;
                }
            };
            let result = storage.submit_alert(
                request.trace_id,
                &request.alert_token,
                &request.admission.plugin_id,
                &draft,
                request.created_at,
            );
            request.admission.complete()?;
            match result {
                Ok(alert_contract::AlertSubmitOutcome::Stored(_))
                | Ok(alert_contract::AlertSubmitOutcome::RejectedTraceToken) => {}
                Err(error) => {
                    issues.push(AlertIngressIssue {
                        trace_id: request.trace_id,
                        instance_id: request.admission.instance_id.clone(),
                        code: error.stage,
                        message: error.message,
                    });
                }
            }
            processed += 1;
        }
        if processed == self.writes_per_cycle {
            let _ = self.signal.notify();
        }
        Ok(issues)
    }

    fn registration(&self, instance_id: &str) -> Result<&Arc<AlertAdmission>, ControlError> {
        self.registrations.get(instance_id).ok_or_else(|| {
            ControlError::new(
                "alert_ingress_registration",
                format!("plugin instance {instance_id} is not registered"),
            )
        })
    }
}

fn resolve_schema_path(manifest_path: &Path, schema_ref: &str) -> PathBuf {
    let path = PathBuf::from(schema_ref);
    if path.is_absolute() {
        return path;
    }
    manifest_path
        .parent()
        .map(|parent| parent.join(&path))
        .unwrap_or(path)
}

fn alert_control_error(error: alert_contract::AlertStoreError) -> ControlError {
    ControlError::new(error.stage, error.message)
}
