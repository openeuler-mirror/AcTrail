//! Trace-scoped fanotify permission enforcement.

use std::collections::{BTreeMap, BTreeSet};
use std::os::fd::RawFd;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};

use crate::services::seccomp_notify::NotificationContinuation;
use collector_capability::CollectorDescriptor;
use config_core::capture_profile::FileEnforcementSeccompRequirements;
use config_core::daemon::{EnforcementConfig, EnforcementDecision};
use control_contract::reply::ControlError;
use ebpf_collector::EbpfCollector;
use ebpf_collector::procfs::ProcfsIdentityReader;
use model_core::capability::{Capability, CapabilityDescriptor, CapabilityField, GuaranteeClass};
use model_core::ids::CollectorName;
use model_core::process::{ProcessIdentity, ProcessRecord};
use plugin_system::{
    FilePolicyApplyRequest, FilePolicyApplyResult, FilePolicyHost, FilePolicyListFilter,
    FilePolicyListResult, FilePolicyMatchDryRunRequest, FilePolicyMatchDryRunResult,
    FilePolicyRulesApplyGrant, PluginRuntimeError,
};
use process_identity::ProcessIdentityManager;
use trace_runtime::registry::TraceRuntime;

#[path = "service/audit.rs"]
mod audit;
#[path = "service/decision.rs"]
mod decision;
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
use super::rules::EnforcementRules;
use crate::services::control_runtime::ControlPluginRuntime;

pub(in crate::services) const COLLECTOR_NAME: &str = "fanotify-enforcement";

pub(in crate::services) struct EnforcementDrain {
    pub(in crate::services) outcomes: Vec<EnforcementOutcomeDraft>,
    pub(in crate::services) process_records: Vec<ProcessRecord>,
}

pub(in crate::services) struct FanotifyEnforcementService {
    backend: Option<Arc<Mutex<FanotifyBackend>>>,
}

#[derive(Clone)]
pub(in crate::services) struct FanotifyFilePolicyHost {
    backend: Option<Arc<Mutex<FanotifyBackend>>>,
    control_plugins: ControlPluginRuntime,
}

impl FanotifyEnforcementService {
    pub(in crate::services) fn disabled() -> Self {
        Self { backend: None }
    }

    pub(in crate::services) fn new(config: EnforcementConfig) -> Result<Self, ControlError> {
        if !config.enabled {
            return Ok(Self::disabled());
        }
        FanotifyBackend::new(config)
            .map(|backend| Self {
                backend: Some(Arc::new(Mutex::new(backend))),
            })
            .map_err(|message| ControlError::new("fanotify_enforcement", message))
    }

    pub(in crate::services) fn enabled(&self) -> bool {
        self.backend.is_some()
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
                .contains(&config_core::daemon::EnforcementSeccompSyscall::Mkdir)
                && backend
                    .rules
                    .has_operation_rules(plugin_system::FilePolicyOperation::Mkdir),
            backend
                .seccomp_syscalls
                .contains(&config_core::daemon::EnforcementSeccompSyscall::Rmdir)
                && backend
                    .rules
                    .has_operation_rules(plugin_system::FilePolicyOperation::Rmdir),
        ))
    }

    pub(in crate::services) fn event_poll_fds(&self) -> Vec<RawFd> {
        self.backend
            .as_ref()
            .and_then(|backend| backend.lock().ok().map(|backend| backend.event_poll_fds()))
            .unwrap_or_default()
    }

    pub(in crate::services) fn file_policy_host(
        &self,
        control_plugins: ControlPluginRuntime,
    ) -> FanotifyFilePolicyHost {
        FanotifyFilePolicyHost {
            backend: self.backend.clone(),
            control_plugins,
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

impl FilePolicyHost for FanotifyFilePolicyHost {
    fn rules_version_get(&self) -> Result<u64, PluginRuntimeError> {
        let backend = self.lock_backend()?;
        Ok(backend.rules.revision())
    }

    fn rules_list(
        &self,
        filter: FilePolicyListFilter,
        cursor: Option<String>,
        limit: u32,
    ) -> Result<FilePolicyListResult, PluginRuntimeError> {
        let backend = self.lock_backend()?;
        Ok(backend.rules.list(filter, cursor.as_deref(), limit))
    }

    fn rules_match_dry_run(
        &self,
        request: FilePolicyMatchDryRunRequest,
    ) -> Result<FilePolicyMatchDryRunResult, PluginRuntimeError> {
        let backend = self.lock_backend()?;
        backend
            .rules
            .match_dry_run(
                request,
                default_file_policy_decision(backend.default_decision),
            )
            .map_err(file_policy_error)
    }

    fn rules_validate(
        &self,
        owner_instance_id: &str,
        grants: &[FilePolicyRulesApplyGrant],
        request: &FilePolicyApplyRequest,
    ) -> Result<FilePolicyApplyResult, PluginRuntimeError> {
        let backend = self.lock_backend()?;
        Ok(backend
            .rules
            .validate_apply(owner_instance_id, grants, request, |target| {
                self.control_plugins.is_instance_index_active(target)
            }))
    }

    fn rules_apply(
        &self,
        owner_instance_id: &str,
        grants: &[FilePolicyRulesApplyGrant],
        request: FilePolicyApplyRequest,
    ) -> Result<FilePolicyApplyResult, PluginRuntimeError> {
        let mut backend = self.lock_backend()?;
        let mut candidate = backend.rules.clone();
        let (result, _) = candidate
            .apply(owner_instance_id, grants, request, |target| {
                self.control_plugins.is_instance_index_active(target)
            })
            .map_err(file_policy_error)?;
        backend
            .replace_rules(candidate)
            .map_err(file_policy_error)?;
        Ok(result)
    }
}

impl FanotifyFilePolicyHost {
    fn lock_backend(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, FanotifyBackend>, PluginRuntimeError> {
        let Some(backend) = &self.backend else {
            return Err(PluginRuntimeError::new(
                "file_policy",
                "fanotify enforcement is disabled",
            ));
        };
        backend.lock().map_err(|error| {
            PluginRuntimeError::new("file_policy", format!("lock backend: {error}"))
        })
    }
}

struct FanotifyBackend {
    handle: FanotifyHandle,
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

impl FanotifyBackend {
    fn new(config: EnforcementConfig) -> Result<Self, String> {
        let mut rules = EnforcementRules::load(&config.rules_path)?;
        rules.install_builtin_allow_rules(
            config
                .builtin_rules
                .iter()
                .map(|rule| (rule.rule_id.clone(), rule.path.clone())),
        )?;
        let handle = FanotifyHandle::new(config.event_buffer_bytes)?;
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
        let wake_fd = WakeFd::new()?;
        let (completion_tx, completion_rx) = mpsc::channel();
        for directory in rules.mark_directories() {
            handle.mark_directory(directory)?;
        }
        Ok(Self {
            handle,
            wake_fd,
            completion_tx,
            completion_rx,
            rules,
            default_decision: config.default_decision,
            audit_enabled: config.audit_enabled,
            reusable_decisions: BTreeMap::new(),
            in_flight_by_rule: BTreeMap::new(),
            in_flight_by_instance: BTreeMap::new(),
            seccomp_syscalls: config.seccomp_syscalls.into_iter().collect(),
            seccomp_path_max_bytes: config.seccomp_path_max_bytes,
        })
    }

    fn event_poll_fds(&self) -> Vec<RawFd> {
        vec![self.handle.fd(), self.wake_fd.fd()]
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

        let fanotify_fd = self.handle.fd();
        let wake_fd = self.wake_fd.fd();
        let rules = &self.rules;
        let default_decision = self.default_decision;
        let audit_enabled = self.audit_enabled;
        let reusable_decisions = &mut self.reusable_decisions;
        let in_flight_by_rule = &mut self.in_flight_by_rule;
        let in_flight_by_instance = &mut self.in_flight_by_instance;
        let completion_tx = self.completion_tx.clone();
        self.handle.drain(|metadata| {
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
        for directory in candidate_directories.difference(&current_directories) {
            if let Err(error) = self.handle.mark_directory(directory) {
                let rollback_errors = installed
                    .iter()
                    .filter_map(|path| self.handle.unmark_directory(path).err())
                    .collect::<Vec<_>>();
                return if rollback_errors.is_empty() {
                    Err(error)
                } else {
                    Err(format!(
                        "{error}; rollback failed: {}",
                        rollback_errors.join("; ")
                    ))
                };
            }
            installed.push(directory.clone());
        }
        self.rules = candidate;
        self.reusable_decisions.clear();
        for directory in current_directories.difference(&candidate_directories) {
            if let Err(error) = self.handle.unmark_directory(directory) {
                tracing::warn!(
                    path = %directory.display(),
                    error = %error,
                    "obsolete fanotify mark could not be removed"
                );
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

fn default_file_policy_decision(
    decision: EnforcementDecision,
) -> plugin_system::FilePolicyDecision {
    match decision {
        EnforcementDecision::Allow => plugin_system::FilePolicyDecision::Allow,
        EnforcementDecision::Deny => plugin_system::FilePolicyDecision::Deny,
    }
}

fn file_policy_error(message: String) -> PluginRuntimeError {
    PluginRuntimeError::new("file_policy", message)
}

pub(in crate::services) fn descriptor() -> CollectorDescriptor {
    CollectorDescriptor {
        name: CollectorName::new(COLLECTOR_NAME),
        capabilities: vec![CapabilityDescriptor::new(
            Capability::EnforcementFilePermissionFanotify,
            vec![
                CapabilityField::new("file_path", GuaranteeClass::AvailableWhenMetadataObservable),
                CapabilityField::new("decision", GuaranteeClass::AvailableWhenMetadataObservable),
                CapabilityField::new("rule_id", GuaranteeClass::AvailableWhenMetadataObservable),
            ],
        )],
        supports_attach_coverage_guard: false,
        supports_existing_pid_attach: true,
    }
}
