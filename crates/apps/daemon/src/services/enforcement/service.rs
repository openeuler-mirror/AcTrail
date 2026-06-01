//! Trace-scoped fanotify permission enforcement.

use std::collections::BTreeMap;
use std::os::fd::RawFd;
use std::path::Path;
use std::time::SystemTime;

use collector_capability::CollectorDescriptor;
use config_core::daemon::{EnforcementConfig, EnforcementDecision};
use control_contract::reply::ControlError;
use ebpf_collector::procfs::ProcfsIdentityReader;
use model_core::capability::{Capability, CapabilityDescriptor, CapabilityField, GuaranteeClass};
use model_core::event::EnforcementPayload;
use model_core::ids::{CollectorName, TraceId};
use model_core::process::{MembershipState, ProcessIdentity, ProcessMembership};
use process_identity_contract::lookup::ProcessIdentityReader;
use trace_runtime::registry::TraceRuntime;

use super::fanotify::{FanotifyHandle, PermissionEventFd, PermissionMetadata, respond};
use super::rules::{EnforcementRule, EnforcementRules, FileKey};

pub(in crate::services) const COLLECTOR_NAME: &str = "fanotify-enforcement";

pub(in crate::services) struct EnforcementEventDraft {
    pub trace_id: TraceId,
    pub observed_at: SystemTime,
    pub process: ProcessIdentity,
    pub payload: EnforcementPayload,
}

pub(in crate::services) struct FanotifyEnforcementService {
    backend: Option<FanotifyBackend>,
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
                backend: Some(backend),
            })
            .map_err(|message| ControlError::new("fanotify_enforcement", message))
    }

    pub(in crate::services) fn enabled(&self) -> bool {
        self.backend.is_some()
    }

    pub(in crate::services) fn event_poll_fd(&self) -> Option<RawFd> {
        self.backend.as_ref().map(FanotifyBackend::event_poll_fd)
    }

    pub(in crate::services) fn drain_due(
        &mut self,
        trace_runtime: &TraceRuntime,
        identity_reader: &ProcfsIdentityReader,
    ) -> Result<Vec<EnforcementEventDraft>, ControlError> {
        match self.backend.as_mut() {
            Some(backend) => backend
                .drain_due(trace_runtime, identity_reader)
                .map_err(|message| ControlError::new("fanotify_enforcement", message)),
            None => Ok(Vec::new()),
        }
    }
}

struct FanotifyBackend {
    handle: FanotifyHandle,
    rules: EnforcementRules,
    default_decision: EnforcementDecision,
    audit_enabled: bool,
}

impl FanotifyBackend {
    fn new(config: EnforcementConfig) -> Result<Self, String> {
        let rules = EnforcementRules::load(&config.rules_path)?;
        if rules.is_empty() {
            return Err(format!(
                "enforcement rules {} must contain at least one rule",
                config.rules_path.display()
            ));
        }
        let handle = FanotifyHandle::new(config.event_buffer_bytes)?;
        for directory in rules.mark_directories() {
            handle.mark_directory(directory)?;
        }
        Ok(Self {
            handle,
            rules,
            default_decision: config.default_decision,
            audit_enabled: config.audit_enabled,
        })
    }

    fn event_poll_fd(&self) -> RawFd {
        self.handle.fd()
    }

    fn drain_due(
        &mut self,
        trace_runtime: &TraceRuntime,
        identity_reader: &ProcfsIdentityReader,
    ) -> Result<Vec<EnforcementEventDraft>, String> {
        let mut drafts = Vec::new();
        let fanotify_fd = self.handle.fd();
        let rules = &self.rules;
        let default_decision = self.default_decision;
        let audit_enabled = self.audit_enabled;
        self.handle.drain(|metadata| {
            handle_permission_event(
                fanotify_fd,
                rules,
                default_decision,
                audit_enabled,
                trace_runtime,
                identity_reader,
                metadata,
                &mut drafts,
            )
        })?;
        Ok(drafts)
    }
}

fn handle_permission_event(
    fanotify_fd: RawFd,
    rules: &EnforcementRules,
    default_decision: EnforcementDecision,
    audit_enabled: bool,
    trace_runtime: &TraceRuntime,
    identity_reader: &ProcfsIdentityReader,
    metadata: PermissionMetadata,
    drafts: &mut Vec<EnforcementEventDraft>,
) -> Result<(), String> {
    let event_fd = PermissionEventFd::new(metadata.fd);
    if metadata.mask & libc::FAN_OPEN_PERM == 0 {
        return Ok(());
    }
    let Some((trace_id, process)) = traced_process(metadata.pid, trace_runtime, identity_reader)?
    else {
        respond(fanotify_fd, event_fd.raw_fd(), true)?;
        return Ok(());
    };
    if !trace_requests_enforcement(trace_runtime, trace_id) {
        respond(fanotify_fd, event_fd.raw_fd(), true)?;
        return Ok(());
    }

    let file_key = event_fd.file_key()?;
    let observed_path = event_fd.display_path();
    let decision = decision_for_path(rules, default_decision, observed_path.as_deref());
    respond(
        fanotify_fd,
        event_fd.raw_fd(),
        matches!(decision.decision, EnforcementDecision::Allow),
    )?;
    if audit_enabled {
        drafts.push(event_draft(
            trace_id,
            process,
            decision,
            file_key,
            observed_path,
        ));
    }
    Ok(())
}

fn traced_process(
    pid: i32,
    trace_runtime: &TraceRuntime,
    identity_reader: &ProcfsIdentityReader,
) -> Result<Option<(TraceId, ProcessIdentity)>, String> {
    let Ok(pid) = u32::try_from(pid) else {
        return Ok(None);
    };
    let identity = match identity_reader.read_identity(pid) {
        Ok(identity) => identity,
        Err(_) => return Ok(None),
    };
    Ok(trace_runtime
        .find_membership(&identity)
        .filter(|(_, membership)| membership_is_capturable(membership))
        .map(|(trace_id, membership)| (trace_id, membership.identity)))
}

fn membership_is_capturable(membership: &ProcessMembership) -> bool {
    membership.capture_enabled
        && matches!(
            membership.state,
            MembershipState::Starting | MembershipState::Active
        )
}

fn trace_requests_enforcement(trace_runtime: &TraceRuntime, trace_id: TraceId) -> bool {
    trace_runtime
        .get_trace(trace_id)
        .map(|entry| {
            entry.sensor_plan.collectors.iter().any(|collector| {
                collector.collector_name.as_str() == COLLECTOR_NAME
                    && collector
                        .capabilities
                        .contains(&Capability::EnforcementFilePermissionFanotify)
            })
        })
        .unwrap_or(false)
}

struct Decision<'a> {
    decision: EnforcementDecision,
    rule: Option<&'a EnforcementRule>,
}

fn decision_for_path<'a>(
    rules: &'a EnforcementRules,
    default_decision: EnforcementDecision,
    path: Option<&str>,
) -> Decision<'a> {
    match path.and_then(|path| rules.find_path(Path::new(path))) {
        Some(rule) => Decision {
            decision: rule.decision,
            rule: Some(rule),
        },
        None => Decision {
            decision: default_decision,
            rule: None,
        },
    }
}

fn event_draft(
    trace_id: TraceId,
    process: ProcessIdentity,
    decision: Decision<'_>,
    file_key: FileKey,
    fallback_path: Option<String>,
) -> EnforcementEventDraft {
    let mut metadata = BTreeMap::from([
        ("scope".to_string(), "trace".to_string()),
        ("file_dev".to_string(), file_key.dev.to_string()),
        ("file_ino".to_string(), file_key.ino.to_string()),
    ]);
    if decision.rule.is_none() {
        metadata.insert("rule_source".to_string(), "default".to_string());
    }
    EnforcementEventDraft {
        trace_id,
        observed_at: SystemTime::now(),
        process,
        payload: EnforcementPayload {
            backend: "fanotify".to_string(),
            operation: "open".to_string(),
            decision: decision.decision.as_str().to_string(),
            path: decision
                .rule
                .map(|rule| rule.path.display().to_string())
                .or(fallback_path),
            rule_id: decision.rule.map(|rule| rule.rule_id.clone()),
            result: match decision.decision {
                EnforcementDecision::Allow => "allowed",
                EnforcementDecision::Deny => "denied",
            }
            .to_string(),
            metadata,
        },
    }
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
