use std::collections::BTreeMap;
use std::os::fd::RawFd;
use std::path::Path;
use std::sync::mpsc::Sender;
use std::thread;

use config_core::daemon::EnforcementDecision;
use ebpf_collector::procfs::ProcfsIdentityReader;
use model_core::capability::Capability;
use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;
use plugin_system::{
    CONTROL_CURRENT_CONTEXT_TOKEN, ControlDecisionBudget, ControlDecisionRequest, ControlSubject,
    FILE_POLICY_CURRENT_CONTEXT_TOKEN, FilePolicyMatchedRule, FilePolicyOperation,
    FilePolicyReadContext,
};
use process_identity::ProcessIdentityManager;
use trace_runtime::registry::TraceRuntime;

use super::audit::{
    Decision, DecisionSource, EnforcementOutcomeDraft, SyncPluginFallbackReason, outcome_draft,
};
use super::worker::{
    CachedDecision, PendingPluginDecision, PluginDecisionCompletion, ReusableDecisionKey,
    complete_plugin_decision,
};
use crate::services::control_runtime::ControlPluginRuntime;
use crate::services::enforcement::fanotify::{PermissionEventFd, PermissionMetadata, respond};
use crate::services::enforcement::rules::{
    EnforcementRule, EnforcementRules, FileKey, RuleDecision,
};
use crate::services::identity::ControlActorIdentityResolver;
use crate::services::identity::TraceIdentityResolver;

pub(super) fn handle_permission_event(
    fanotify_fd: RawFd,
    wake_fd: RawFd,
    rules: &EnforcementRules,
    default_decision: EnforcementDecision,
    audit_enabled: bool,
    trace_runtime: &TraceRuntime,
    process_registry: &ProcessIdentityManager,
    identity_reader: &ProcfsIdentityReader,
    control_plugins: &ControlPluginRuntime,
    reusable_decisions: &mut BTreeMap<ReusableDecisionKey, CachedDecision>,
    in_flight_by_rule: &mut BTreeMap<String, u32>,
    in_flight_by_instance: &mut BTreeMap<String, u32>,
    completion_tx: Sender<PluginDecisionCompletion>,
    metadata: PermissionMetadata,
    drafts: &mut Vec<EnforcementOutcomeDraft>,
) -> Result<(), String> {
    let event_fd = PermissionEventFd::new(metadata.fd);
    if metadata.mask & libc::FAN_OPEN_PERM == 0 {
        return Ok(());
    }
    let observed_path = event_fd.display_path();
    if observed_path
        .as_deref()
        .and_then(|path| rules.find_builtin_allow(Path::new(path)))
        .is_some()
    {
        respond(fanotify_fd, event_fd.raw_fd(), true)?;
        return Ok(());
    }
    let Some((trace_id, process)) = traced_process(
        metadata.pid,
        trace_runtime,
        process_registry,
        identity_reader,
    )?
    else {
        respond(fanotify_fd, event_fd.raw_fd(), true)?;
        return Ok(());
    };
    if !trace_requests_enforcement(trace_runtime, trace_id) {
        respond(fanotify_fd, event_fd.raw_fd(), true)?;
        return Ok(());
    }

    let Some(rule) = observed_path
        .as_deref()
        .and_then(|path| rules.find_path(FilePolicyOperation::Open, Path::new(path)))
    else {
        let decision = Decision {
            decision: default_decision,
            rule: None,
            source: DecisionSource::Default,
        };
        return respond_and_capture(
            fanotify_fd,
            event_fd,
            trace_id,
            process,
            decision,
            audit_enabled,
            observed_path,
            drafts,
        );
    };
    handle_rule_permission_event(
        fanotify_fd,
        wake_fd,
        event_fd,
        rule,
        trace_id,
        process,
        process_registry,
        control_plugins,
        reusable_decisions,
        in_flight_by_rule,
        in_flight_by_instance,
        completion_tx,
        audit_enabled,
        observed_path,
        drafts,
    )
}

pub(super) fn prune_reusable_decisions(
    trace_runtime: &TraceRuntime,
    reusable_decisions: &mut BTreeMap<ReusableDecisionKey, CachedDecision>,
) {
    reusable_decisions.retain(|key, _| {
        trace_runtime
            .get_trace(key.trace_id)
            .is_some_and(|entry| !entry.trace.lifecycle_state.is_terminal())
    });
}

fn traced_process(
    pid: i32,
    trace_runtime: &TraceRuntime,
    process_registry: &ProcessIdentityManager,
    identity_reader: &ProcfsIdentityReader,
) -> Result<Option<(TraceId, ProcessIdentity)>, String> {
    let Ok(pid) = u32::try_from(pid) else {
        return Ok(None);
    };
    let resolved = match TraceIdentityResolver::new(trace_runtime, process_registry)
        .read_and_match_pid(identity_reader, pid, "fanotify_identity")
    {
        Ok(resolved) => resolved,
        Err(_) => return Ok(None),
    };
    Ok(resolved
        .filter(|process| process.is_capturable())
        .map(|process| (process.trace_id, process.process)))
}

pub(super) fn trace_requests_enforcement(trace_runtime: &TraceRuntime, trace_id: TraceId) -> bool {
    trace_runtime
        .get_trace(trace_id)
        .map(|entry| {
            entry.sensor_plan.collectors.iter().any(|collector| {
                collector.collector_name.as_str() == super::COLLECTOR_NAME
                    && collector
                        .capabilities
                        .contains(&Capability::EnforcementFilePermissionFanotify)
            })
        })
        .unwrap_or(false)
}

fn handle_rule_permission_event(
    fanotify_fd: RawFd,
    wake_fd: RawFd,
    event_fd: PermissionEventFd,
    rule: &EnforcementRule,
    trace_id: TraceId,
    process: ProcessIdentity,
    process_registry: &ProcessIdentityManager,
    control_plugins: &ControlPluginRuntime,
    reusable_decisions: &mut BTreeMap<ReusableDecisionKey, CachedDecision>,
    in_flight_by_rule: &mut BTreeMap<String, u32>,
    in_flight_by_instance: &mut BTreeMap<String, u32>,
    completion_tx: Sender<PluginDecisionCompletion>,
    audit_enabled: bool,
    observed_path: Option<String>,
    drafts: &mut Vec<EnforcementOutcomeDraft>,
) -> Result<(), String> {
    match &rule.decision {
        RuleDecision::Local(decision) => respond_and_capture(
            fanotify_fd,
            event_fd,
            trace_id,
            process,
            Decision {
                decision: *decision,
                rule: Some(rule),
                source: DecisionSource::Rule,
            },
            audit_enabled,
            observed_path,
            drafts,
        ),
        RuleDecision::SyncPlugin {
            instance_id,
            timeout_ms,
            concurrency_limit,
            fallback,
        } => handle_sync_plugin_rule(
            fanotify_fd,
            wake_fd,
            event_fd,
            rule,
            trace_id,
            process,
            process_registry,
            control_plugins,
            reusable_decisions,
            in_flight_by_rule,
            in_flight_by_instance,
            completion_tx,
            audit_enabled,
            observed_path,
            instance_id,
            *timeout_ms,
            *concurrency_limit,
            *fallback,
            drafts,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_sync_plugin_rule(
    fanotify_fd: RawFd,
    wake_fd: RawFd,
    event_fd: PermissionEventFd,
    rule: &EnforcementRule,
    trace_id: TraceId,
    process: ProcessIdentity,
    process_registry: &ProcessIdentityManager,
    control_plugins: &ControlPluginRuntime,
    reusable_decisions: &mut BTreeMap<ReusableDecisionKey, CachedDecision>,
    in_flight_by_rule: &mut BTreeMap<String, u32>,
    in_flight_by_instance: &mut BTreeMap<String, u32>,
    completion_tx: Sender<PluginDecisionCompletion>,
    audit_enabled: bool,
    observed_path: Option<String>,
    instance_id: &str,
    timeout_ms: u64,
    concurrency_limit: u32,
    fallback: EnforcementDecision,
    drafts: &mut Vec<EnforcementOutcomeDraft>,
) -> Result<(), String> {
    let cache_key = ReusableDecisionKey {
        trace_id,
        rule_id: rule.rule_id.clone(),
        operation: FilePolicyOperation::Open,
    };
    if let Some(cached) = reusable_decisions.get(&cache_key) {
        return respond_and_capture(
            fanotify_fd,
            event_fd,
            trace_id,
            process,
            Decision {
                decision: cached.decision,
                rule: Some(rule),
                source: DecisionSource::SyncPluginCache {
                    instance_id: instance_id.to_string(),
                },
            },
            audit_enabled,
            observed_path,
            drafts,
        );
    }

    let in_flight = in_flight_by_rule
        .get(&rule.rule_id)
        .copied()
        .unwrap_or_default();
    if in_flight >= concurrency_limit {
        return respond_and_capture(
            fanotify_fd,
            event_fd,
            trace_id,
            process,
            Decision {
                decision: fallback,
                rule: Some(rule),
                source: DecisionSource::SyncPluginFallback {
                    instance_id: instance_id.to_string(),
                    timeout_ms,
                    concurrency_limit,
                    reason: SyncPluginFallbackReason::ConcurrencyLimit,
                    error: None,
                    in_flight: Some(in_flight),
                    instance_concurrency_limit: None,
                    instance_in_flight: None,
                    fallback,
                },
            },
            audit_enabled,
            observed_path,
            drafts,
        );
    }

    let instance_in_flight = in_flight_by_instance
        .get(instance_id)
        .copied()
        .unwrap_or_default();
    let instance_concurrency_limit = control_plugins
        .instance_concurrency_limit(instance_id)
        .unwrap_or(concurrency_limit);
    if instance_in_flight >= instance_concurrency_limit {
        return respond_and_capture(
            fanotify_fd,
            event_fd,
            trace_id,
            process,
            Decision {
                decision: fallback,
                rule: Some(rule),
                source: DecisionSource::SyncPluginFallback {
                    instance_id: instance_id.to_string(),
                    timeout_ms,
                    concurrency_limit,
                    reason: SyncPluginFallbackReason::PluginInstanceConcurrencyLimit,
                    error: None,
                    in_flight: None,
                    instance_concurrency_limit: Some(instance_concurrency_limit),
                    instance_in_flight: Some(instance_in_flight),
                    fallback,
                },
            },
            audit_enabled,
            observed_path,
            drafts,
        );
    }

    reserve_in_flight(in_flight_by_rule, &rule.rule_id)?;
    if let Err(error) = reserve_in_flight(in_flight_by_instance, instance_id) {
        release_in_flight(in_flight_by_rule, &rule.rule_id);
        return Err(error);
    }
    let (file_key, audit_metadata_error) = if audit_enabled {
        audit_metadata(&event_fd)
    } else {
        (None, None)
    };
    let request = control_decision_request(
        trace_id,
        &process,
        process_registry,
        rule,
        FilePolicyOperation::Open,
        observed_path.as_deref(),
    )?;
    let pending = PendingPluginDecision {
        fanotify_fd,
        wake_fd,
        event_fd,
        trace_id,
        process,
        rule: rule.clone(),
        instance_id: instance_id.to_string(),
        timeout_ms,
        concurrency_limit,
        fallback,
        request,
        budget: ControlDecisionBudget {
            timeout_ms: Some(timeout_ms),
        },
        control_plugins: control_plugins.clone(),
        audit_enabled,
        observed_path,
        file_key,
        audit_metadata_error,
        cache_key,
        completion_tx,
    };
    thread::spawn(move || complete_plugin_decision(pending));
    Ok(())
}

fn respond_and_capture(
    fanotify_fd: RawFd,
    event_fd: PermissionEventFd,
    trace_id: TraceId,
    process: ProcessIdentity,
    decision: Decision<'_>,
    audit_enabled: bool,
    observed_path: Option<String>,
    drafts: &mut Vec<EnforcementOutcomeDraft>,
) -> Result<(), String> {
    respond(
        fanotify_fd,
        event_fd.raw_fd(),
        matches!(decision.decision, EnforcementDecision::Allow),
    )?;
    let (file_key, audit_metadata_error) = if audit_enabled {
        audit_metadata(&event_fd)
    } else {
        (None, None)
    };
    if let Some(draft) = outcome_draft(
        trace_id,
        process,
        decision,
        audit_enabled,
        FilePolicyOperation::Open,
        file_key,
        observed_path,
        audit_metadata_error,
    ) {
        drafts.push(draft);
    }
    Ok(())
}

fn audit_metadata(event_fd: &PermissionEventFd) -> (Option<FileKey>, Option<String>) {
    match event_fd.file_key() {
        Ok(file_key) => (Some(file_key), None),
        Err(error) => (None, Some(error)),
    }
}

pub(super) fn reserve_in_flight(
    in_flight_by_rule: &mut BTreeMap<String, u32>,
    rule_id: &str,
) -> Result<(), String> {
    let entry = in_flight_by_rule.entry(rule_id.to_string()).or_default();
    *entry = entry
        .checked_add(1)
        .ok_or_else(|| format!("in-flight counter overflow for rule {rule_id}"))?;
    Ok(())
}

pub(super) fn release_in_flight(in_flight_by_rule: &mut BTreeMap<String, u32>, rule_id: &str) {
    match in_flight_by_rule.get_mut(rule_id) {
        Some(count) if *count > 1 => *count -= 1,
        Some(_) => {
            in_flight_by_rule.remove(rule_id);
        }
        None => {}
    }
}

pub(super) fn control_decision_request(
    trace_id: TraceId,
    process: &ProcessIdentity,
    process_registry: &ProcessIdentityManager,
    rule: &EnforcementRule,
    operation: FilePolicyOperation,
    observed_path: Option<&str>,
) -> Result<ControlDecisionRequest, String> {
    let file_policy_context = current_file_access_match_context(rule);
    Ok(ControlDecisionRequest {
        decision_id: format!("{}:{trace_id}", rule.rule_id),
        trace_id: trace_id.to_string(),
        subject: ControlSubject::FileAccess,
        actor_process_identity: ControlActorIdentityResolver::new(process_registry)
            .resolve(*process)
            .map_err(|error| error.message)?,
        operation: operation.as_str().to_string(),
        target_summary: observed_path
            .map(str::to_string)
            .unwrap_or_else(|| rule.path.display().to_string()),
        context_ref: Some(CONTROL_CURRENT_CONTEXT_TOKEN.to_string()),
        file_policy_context,
    })
}

fn current_file_access_match_context(rule: &EnforcementRule) -> Option<FilePolicyReadContext> {
    let RuleDecision::SyncPlugin {
        instance_id,
        timeout_ms,
        concurrency_limit,
        fallback,
    } = &rule.decision
    else {
        return None;
    };
    Some(FilePolicyReadContext {
        context_ref: FILE_POLICY_CURRENT_CONTEXT_TOKEN.to_string(),
        matched_rule: FilePolicyMatchedRule {
            rule_id: rule.rule_id.clone(),
            decision: "gray".to_string(),
            operation: rule.operation.clone(),
            path: rule.path.display().to_string(),
            plugin_instance: Some(instance_id.clone()),
            timeout_ms: Some(*timeout_ms),
            concurrency_limit: Some(*concurrency_limit),
            fallback: Some(fallback.as_str().to_string()),
        },
    })
}
