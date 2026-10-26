//! Trace-scoped fanotify permission enforcement.

use std::collections::{BTreeMap, BTreeSet};
use std::os::fd::RawFd;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};

use crate::services::seccomp_notify::NotificationContinuation;
use collector_capability::CollectorDescriptor;
use config_core::capture_profile::FileEnforcementSeccompRequirements;
use config_core::daemon::{EnforcementConfig, EnforcementDecision, FileControlConfig};
use control_contract::reply::ControlError;
use ebpf_collector::EbpfCollector;
use ebpf_collector::procfs::ProcfsIdentityReader;
use model_core::capability::{Capability, CapabilityDescriptor, CapabilityField, GuaranteeClass};
use model_core::ids::CollectorName;
use model_core::process::{ProcessIdentity, ProcessRecord};
use process_identity::ProcessIdentityManager;
use trace_runtime::registry::TraceRuntime;

#[path = "service/audit.rs"]
mod audit;
#[path = "service/decision.rs"]
mod decision;
#[path = "service/host.rs"]
mod host;
#[path = "service/seccomp.rs"]
mod seccomp;
#[path = "service/wake.rs"]
mod wake;
#[path = "service/worker.rs"]
mod worker;

pub(in crate::services) use self::audit::EnforcementOutcomeDraft;
use self::decision::{handle_permission_event, prune_reusable_decisions, release_in_flight};
use self::wake::WakeFd;
use self::worker::{CachedDecision, PluginDecisionCompletion, ReusableDecisionKey};
use super::fanotify::FanotifyHandle;
use crate::services::control_runtime::ControlPluginRuntime;
use access_control::policy::file::EnforcementRules;

pub(in crate::services) const COLLECTOR_NAME: &str = "file-enforcement";

pub(in crate::services) struct EnforcementDrain {
    pub(in crate::services) outcomes: Vec<EnforcementOutcomeDraft>,
    pub(in crate::services) process_records: Vec<ProcessRecord>,
}

pub(in crate::services) struct FileEnforcementService {
    backend: Option<Arc<Mutex<FileBackend>>>,
}

#[derive(Clone)]
pub(in crate::services) struct FilePolicyServiceHost {
    backend: Option<Arc<Mutex<FileBackend>>>,
}

impl FileEnforcementService {
    pub(in crate::services) fn disabled() -> Self {
        Self { backend: None }
    }

    pub(in crate::services) fn new(
        config: EnforcementConfig,
        operations: FileControlConfig,
    ) -> Result<Self, ControlError> {
        if !config.enabled || !operations.enabled() {
            return Ok(Self::disabled());
        }
        FileBackend::new(config, operations)
            .map(|backend| Self {
                backend: Some(Arc::new(Mutex::new(backend))),
            })
            .map_err(|message| ControlError::new("fanotify_enforcement", message))
    }

    pub(in crate::services) fn seccomp_requirements(
        &self,
    ) -> Result<FileEnforcementSeccompRequirements, ControlError> {
        let Some(backend) = &self.backend else {
            return Ok(FileEnforcementSeccompRequirements::default());
        };
        let backend = backend.lock().map_err(|error| {
            ControlError::new("file_enforcement", format!("lock backend: {error}"))
        })?;
        Ok(FileEnforcementSeccompRequirements::new(
            backend
                .seccomp_syscalls
                .contains(&config_core::daemon::EnforcementSeccompSyscall::Mkdir),
            backend
                .seccomp_syscalls
                .contains(&config_core::daemon::EnforcementSeccompSyscall::Rmdir),
        ))
    }

    pub(in crate::services) fn bind_lsm(
        &self,
        controller: Arc<access_control::lsm::LsmController>,
    ) -> Result<(), ControlError> {
        if let Some(backend) = &self.backend {
            let mut backend = backend.lock().map_err(|error| {
                ControlError::new("file_enforcement", format!("lock backend: {error}"))
            })?;
            controller
                .publish_file_rules(&backend.rules)
                .map_err(|message| ControlError::new("file_policy", message))?;
            backend.lsm = Some(controller);
        }
        Ok(())
    }

    pub(in crate::services) fn event_poll_fds(&self) -> Vec<RawFd> {
        self.backend
            .as_ref()
            .and_then(|backend| backend.lock().ok().map(|backend| backend.event_poll_fds()))
            .unwrap_or_default()
    }

    pub(in crate::services) fn file_policy_host(&self) -> FilePolicyServiceHost {
        FilePolicyServiceHost {
            backend: self.backend.clone(),
        }
    }

    pub(in crate::services) fn remove_plugin_policy_owner(
        &self,
        instance_id: &str,
    ) -> Result<(), ControlError> {
        let Some(backend) = &self.backend else {
            return Ok(());
        };
        let mut backend = backend.lock().map_err(|error| {
            ControlError::new("fanotify_enforcement", format!("lock backend: {error}"))
        })?;
        backend
            .remove_policy_owner(instance_id)
            .map_err(|message| ControlError::new("fanotify_enforcement", message))
    }

    pub(in crate::services) fn drain_due(
        &mut self,
        trace_runtime: &TraceRuntime,
        process_registry: &mut ProcessIdentityManager,
        identity_reader: &ProcfsIdentityReader,
        collector: &EbpfCollector,
        control_plugins: &ControlPluginRuntime,
    ) -> Result<EnforcementDrain, ControlError> {
        match self.backend.as_mut() {
            Some(backend) => backend
                .lock()
                .map_err(|error| {
                    ControlError::new("fanotify_enforcement", format!("lock backend: {error}"))
                })?
                .drain_due(
                    trace_runtime,
                    process_registry,
                    identity_reader,
                    collector,
                    control_plugins,
                )
                .map_err(|message| ControlError::new("fanotify_enforcement", message)),
            None => Ok(EnforcementDrain {
                outcomes: Vec::new(),
                process_records: Vec::new(),
            }),
        }
    }

    pub(in crate::services) fn handle_seccomp_notification(
        &mut self,
        trace_runtime: &TraceRuntime,
        process_registry: &ProcessIdentityManager,
        identity_reader: &ProcfsIdentityReader,
        control_plugins: &ControlPluginRuntime,
        notification: &libc::seccomp_notif,
        continuation: &mut NotificationContinuation,
    ) -> Result<Option<EnforcementOutcomeDraft>, ControlError> {
        let Some(backend) = self.backend.as_mut() else {
            return Ok(None);
        };
        backend
            .lock()
            .map_err(|error| {
                ControlError::new("file_enforcement", format!("lock backend: {error}"))
            })?
            .handle_seccomp_notification(
                trace_runtime,
                process_registry,
                identity_reader,
                control_plugins,
                notification,
                continuation,
            )
    }
}

struct FileBackend {
    handle: Option<FanotifyHandle>,
    lsm: Option<Arc<access_control::lsm::LsmController>>,
    wake_fd: WakeFd,
    completion_tx: Sender<PluginDecisionCompletion>,
    completion_rx: Receiver<PluginDecisionCompletion>,
    rules: EnforcementRules,
    default_decision: EnforcementDecision,
    audit_enabled: bool,
    reusable_decisions: BTreeMap<ReusableDecisionKey, CachedDecision>,
    in_flight_by_rule: BTreeMap<String, u32>,
    in_flight_by_instance: BTreeMap<String, u32>,
    seccomp_syscalls: std::collections::BTreeSet<config_core::daemon::EnforcementSeccompSyscall>,
    seccomp_path_max_bytes: u32,
}

impl FileBackend {
    fn new(config: EnforcementConfig, operations: FileControlConfig) -> Result<Self, String> {
        let mut rules = if operations.open.is_fanotify() {
            EnforcementRules::load(&config.rules_path)?
        } else {
            EnforcementRules::load_for_lsm(&config.rules_path)?
        };
        rules.install_builtin_allow_rules(
            config
                .builtin_rules
                .iter()
                .map(|rule| (rule.rule_id.clone(), rule.path.clone())),
        )?;
        let handle = if operations.open.is_fanotify() {
            Some(FanotifyHandle::new(config.event_buffer_bytes)?)
        } else {
            None
        };
        if let Some(handle) = &handle {
            for mark in rules.builtin_ignore_marks() {
                if !mark.path.exists() {
                    tracing::debug!(
                        path = %mark.path.display(),
                        "builtin fanotify ignore path does not exist yet"
                    );
                    continue;
                }
                if let Err(error) = handle.ignore_path(&mark.path, mark.recursive) {
                    tracing::warn!(
                        path = %mark.path.display(),
                        error = %error,
                        "builtin fanotify ignore mark failed; user-space builtin allow remains active"
                    );
                }
            }
        }
        let wake_fd = WakeFd::new()?;
        let (completion_tx, completion_rx) = mpsc::channel();
        if let Some(handle) = &handle {
            for directory in rules.mark_directories() {
                handle.mark_directory(directory)?;
            }
        }
        Ok(Self {
            handle,
            lsm: None,
            wake_fd,
            completion_tx,
            completion_rx,
            rules,
            default_decision: config.default_decision,
            audit_enabled: config.audit_enabled,
            reusable_decisions: BTreeMap::new(),
            in_flight_by_rule: BTreeMap::new(),
            in_flight_by_instance: BTreeMap::new(),
            seccomp_syscalls: operations.seccomp_syscalls().into_iter().collect(),
            seccomp_path_max_bytes: config.seccomp_path_max_bytes,
        })
    }

    fn event_poll_fds(&self) -> Vec<RawFd> {
        let mut fds = vec![self.wake_fd.fd()];
        if let Some(handle) = &self.handle {
            fds.push(handle.fd());
        }
        fds
    }

    fn drain_due(
        &mut self,
        trace_runtime: &TraceRuntime,
        process_registry: &mut ProcessIdentityManager,
        identity_reader: &ProcfsIdentityReader,
        collector: &EbpfCollector,
        control_plugins: &ControlPluginRuntime,
    ) -> Result<EnforcementDrain, String> {
        let mut drafts = Vec::new();
        let mut fork_identity_records = BTreeMap::<ProcessIdentity, ProcessRecord>::new();
        self.wake_fd.drain()?;
        self.drain_plugin_completions(&mut drafts)?;
        prune_reusable_decisions(trace_runtime, &mut self.reusable_decisions);

        let Some(handle) = &self.handle else {
            return Ok(EnforcementDrain {
                outcomes: drafts,
                process_records: Vec::new(),
            });
        };
        let fanotify_fd = handle.fd();
        let wake_fd = self.wake_fd.fd();
        let rules = &self.rules;
        let default_decision = self.default_decision;
        let audit_enabled = self.audit_enabled;
        let reusable_decisions = &mut self.reusable_decisions;
        let in_flight_by_rule = &mut self.in_flight_by_rule;
        let in_flight_by_instance = &mut self.in_flight_by_instance;
        let completion_tx = self.completion_tx.clone();
        handle.drain(|metadata| {
            handle_permission_event(
                fanotify_fd,
                wake_fd,
                rules,
                default_decision,
                audit_enabled,
                trace_runtime,
                process_registry,
                identity_reader,
                collector,
                control_plugins,
                reusable_decisions,
                in_flight_by_rule,
                in_flight_by_instance,
                completion_tx.clone(),
                metadata,
                &mut drafts,
                &mut fork_identity_records,
            )
        })?;
        self.drain_plugin_completions(&mut drafts)?;
        Ok(EnforcementDrain {
            outcomes: drafts,
            process_records: fork_identity_records.into_values().collect(),
        })
    }

    fn drain_plugin_completions(
        &mut self,
        drafts: &mut Vec<EnforcementOutcomeDraft>,
    ) -> Result<(), String> {
        loop {
            match self.completion_rx.try_recv() {
                Ok(completion) => {
                    release_in_flight(&mut self.in_flight_by_rule, &completion.rule_id);
                    release_in_flight(&mut self.in_flight_by_instance, &completion.instance_id);
                    if let Some((key, decision)) = completion.cache_update {
                        self.reusable_decisions.insert(key, decision);
                    }
                    if let Some(draft) = completion.draft {
                        drafts.push(draft);
                    }
                    if let Some(error) = completion.error {
                        return Err(error);
                    }
                }
                Err(TryRecvError::Empty) => return Ok(()),
                Err(TryRecvError::Disconnected) => {
                    return Err(
                        "fanotify control plugin completion channel disconnected".to_string()
                    );
                }
            }
        }
    }

    fn replace_rules(&mut self, candidate: EnforcementRules) -> Result<(), String> {
        let current_directories = self
            .rules
            .mark_directories()
            .cloned()
            .collect::<BTreeSet<_>>();
        let candidate_directories = candidate
            .mark_directories()
            .cloned()
            .collect::<BTreeSet<_>>();
        let mut installed = Vec::<std::path::PathBuf>::new();
        let prepared = (|| {
            if let Some(handle) = &self.handle {
                for directory in candidate_directories.difference(&current_directories) {
                    handle.mark_directory(directory)?;
                    installed.push(directory.clone());
                }
            }
            if let Some(lsm) = &self.lsm {
                lsm.publish_file_rules(&candidate)?;
            }
            Ok::<(), String>(())
        })();
        if let Err(error) = prepared {
            let rollback_errors = self
                .handle
                .as_ref()
                .map(|handle| {
                    installed
                        .iter()
                        .filter_map(|path| handle.unmark_directory(path).err())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            return if rollback_errors.is_empty() {
                Err(error)
            } else {
                Err(format!(
                    "{error}; rollback failed: {}",
                    rollback_errors.join("; ")
                ))
            };
        }
        self.rules = candidate;
        self.reusable_decisions.clear();
        if let Some(handle) = &self.handle {
            for directory in current_directories.difference(&candidate_directories) {
                if let Err(error) = handle.unmark_directory(directory) {
                    tracing::warn!(
                        path = %directory.display(),
                        error = %error,
                        "obsolete fanotify mark could not be removed"
                    );
                }
            }
        }
        Ok(())
    }

    fn remove_policy_owner(&mut self, instance_id: &str) -> Result<(), String> {
        let mut candidate = self.rules.clone();
        candidate.remove_owner(instance_id)?;
        self.replace_rules(candidate)
    }
}

pub(in crate::services) fn descriptor(operations: &FileControlConfig) -> CollectorDescriptor {
    let mut capabilities = Vec::new();
    for (enabled, capability) in [
        (
            operations.open.is_fanotify(),
            Capability::EnforcementFilePermissionFanotify,
        ),
        (
            operations.mkdir.is_seccomp(),
            Capability::EnforcementFileMkdirSeccomp,
        ),
        (
            operations.rmdir.is_seccomp(),
            Capability::EnforcementFileRmdirSeccomp,
        ),
    ] {
        if enabled {
            capabilities.push(CapabilityDescriptor::new(
                capability,
                vec![
                    CapabilityField::new(
                        "file_path",
                        GuaranteeClass::AvailableWhenMetadataObservable,
                    ),
                    CapabilityField::new(
                        "decision",
                        GuaranteeClass::AvailableWhenMetadataObservable,
                    ),
                    CapabilityField::new(
                        "rule_id",
                        GuaranteeClass::AvailableWhenMetadataObservable,
                    ),
                ],
            ));
        }
    }
    CollectorDescriptor {
        name: CollectorName::new(COLLECTOR_NAME),
        capabilities,
        supports_attach_coverage_guard: false,
        supports_existing_pid_attach: true,
    }
}
