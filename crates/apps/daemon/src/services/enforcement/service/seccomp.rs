use std::collections::BTreeSet;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Component, Path, PathBuf};

use config_core::daemon::{EnforcementDecision, EnforcementSeccompSyscall};
use control_contract::reply::ControlError;
use linux_platform::file_seccomp::KernelFileSeccompSyscall;
use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;
use plugin_system::{ControlDecisionBudget, ControlVerdict, DecisionScope, FilePolicyOperation};
use process_identity::{ProcessIdentityManager, ProcessIdentityReader};
use trace_runtime::registry::TraceRuntime;

use super::FanotifyBackend;
use super::audit::{
    Decision, DecisionSource, EnforcementOutcomeDraft, SyncPluginFallbackReason, outcome_draft,
};
use super::decision::{
    control_decision_request, release_in_flight, reserve_in_flight, trace_requests_enforcement,
};
use super::worker::{CachedDecision, ReusableDecisionKey};
use crate::services::control_runtime::ControlPluginRuntime;
use crate::services::enforcement::rules::{EnforcementRule, RuleDecision};
use crate::services::identity::TraceIdentityResolver;
use crate::services::seccomp_notify::{NotificationContinuation, read_c_string, target_exited};

impl FanotifyBackend {
    pub(super) fn handle_seccomp_notification(
        &mut self,
        trace_runtime: &TraceRuntime,
        process_registry: &ProcessIdentityManager,
        identity_reader: &impl ProcessIdentityReader,
        control_plugins: &ControlPluginRuntime,
        notification: &libc::seccomp_notif,
        continuation: &mut NotificationContinuation,
    ) -> Result<Option<EnforcementOutcomeDraft>, ControlError> {
        let Some(request) =
            MutationRequest::from_notification(notification, &self.seccomp_syscalls)
        else {
            return Ok(None);
        };
        if self.default_decision == EnforcementDecision::Allow
            && !self.rules.has_operation_rules(request.operation)
        {
            return Ok(None);
        }
        let path = match request.absolute_path(self.seccomp_path_max_bytes) {
            Ok(Some(path)) => path,
            Ok(None) => return Ok(None),
            Err(error) => {
                continuation.deny_errno(libc::EACCES)?;
                tracing::warn!(
                    error.code = %error.code,
                    error.message = %error.message,
                    "file mutation denied because its target path could not be resolved"
                );
                return Ok(None);
            }
        };
        let Some((trace_id, process)) = traced_process(
            trace_runtime,
            process_registry,
            identity_reader,
            notification.pid,
        )?
        else {
            return Ok(None);
        };
        if !trace_requests_enforcement(trace_runtime, trace_id) {
            return Ok(None);
        }
        let rule = self.rules.find_path(request.operation, &path).cloned();
        let decision = self.decide_mutation(
            trace_id,
            process,
            process_registry,
            control_plugins,
            request.operation,
            &path,
            rule.as_ref(),
        )?;
        if decision.decision == EnforcementDecision::Deny {
            continuation.deny_errno(libc::EACCES)?;
        } else {
            continuation.continue_now()?;
        }
        Ok(outcome_draft(
            trace_id,
            process,
            decision,
            self.audit_enabled,
            request.operation,
            None,
            Some(path.display().to_string()),
            None,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    fn decide_mutation<'a>(
        &mut self,
        trace_id: TraceId,
        process: ProcessIdentity,
        process_registry: &ProcessIdentityManager,
        control_plugins: &ControlPluginRuntime,
        operation: FilePolicyOperation,
        path: &Path,
        rule: Option<&'a EnforcementRule>,
    ) -> Result<Decision<'a>, ControlError> {
        let Some(rule) = rule else {
            return Ok(Decision {
                decision: self.default_decision,
                rule: None,
                source: DecisionSource::Default,
            });
        };
        match &rule.decision {
            RuleDecision::Local(decision) => Ok(Decision {
                decision: *decision,
                rule: Some(rule),
                source: DecisionSource::Rule,
            }),
            RuleDecision::SyncPlugin {
                instance_id,
                timeout_ms,
                concurrency_limit,
                fallback,
            } => self.decide_mutation_with_plugin(
                trace_id,
                process,
                process_registry,
                control_plugins,
                operation,
                path,
                rule,
                instance_id,
                *timeout_ms,
                *concurrency_limit,
                *fallback,
            ),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn decide_mutation_with_plugin<'a>(
        &mut self,
        trace_id: TraceId,
        process: ProcessIdentity,
        process_registry: &ProcessIdentityManager,
        control_plugins: &ControlPluginRuntime,
        operation: FilePolicyOperation,
        path: &Path,
        rule: &'a EnforcementRule,
        instance_id: &str,
        timeout_ms: u64,
        concurrency_limit: u32,
        fallback: EnforcementDecision,
    ) -> Result<Decision<'a>, ControlError> {
        let cache_key = ReusableDecisionKey {
            trace_id,
            rule_id: rule.rule_id.clone(),
            operation,
        };
        if let Some(cached) = self.reusable_decisions.get(&cache_key) {
            return Ok(Decision {
                decision: cached.decision,
                rule: Some(rule),
                source: DecisionSource::SyncPluginCache {
                    instance_id: instance_id.to_string(),
                },
            });
        }
        let in_flight = self
            .in_flight_by_rule
            .get(&rule.rule_id)
            .copied()
            .unwrap_or_default();
        if in_flight >= concurrency_limit {
            return Ok(fallback_decision(
                rule,
                instance_id,
                timeout_ms,
                concurrency_limit,
                fallback,
                SyncPluginFallbackReason::ConcurrencyLimit,
                None,
                Some(in_flight),
            ));
        }
        let instance_in_flight = self
            .in_flight_by_instance
            .get(instance_id)
            .copied()
            .unwrap_or_default();
        let instance_limit = control_plugins
            .instance_concurrency_limit(instance_id)
            .unwrap_or(concurrency_limit);
        if instance_in_flight >= instance_limit {
            return Ok(Decision {
                decision: fallback,
                rule: Some(rule),
                source: DecisionSource::SyncPluginFallback {
                    instance_id: instance_id.to_string(),
                    timeout_ms,
                    concurrency_limit,
                    reason: SyncPluginFallbackReason::PluginInstanceConcurrencyLimit,
                    error: None,
                    in_flight: None,
                    instance_concurrency_limit: Some(instance_limit),
                    instance_in_flight: Some(instance_in_flight),
                    fallback,
                },
            });
        }
        reserve_in_flight(&mut self.in_flight_by_rule, &rule.rule_id)
            .map_err(|message| ControlError::new("file_enforcement", message))?;
        if let Err(message) = reserve_in_flight(&mut self.in_flight_by_instance, instance_id) {
            release_in_flight(&mut self.in_flight_by_rule, &rule.rule_id);
            return Err(ControlError::new("file_enforcement", message));
        }
        let request = control_decision_request(
            trace_id,
            &process,
            process_registry,
            rule,
            operation,
            path.to_str(),
        )
        .map_err(|message| ControlError::new("file_enforcement", message));
        let response = request.and_then(|request| {
            catch_unwind(AssertUnwindSafe(|| {
                control_plugins.decide(
                    instance_id,
                    request,
                    ControlDecisionBudget {
                        timeout_ms: Some(timeout_ms),
                    },
                )
            }))
            .map_err(|_| ControlError::new("file_enforcement", "control plugin panicked"))?
            .map_err(|error| {
                ControlError::new(
                    "file_enforcement",
                    format!("{}: {}", error.code, error.message),
                )
            })
        });
        release_in_flight(&mut self.in_flight_by_rule, &rule.rule_id);
        release_in_flight(&mut self.in_flight_by_instance, instance_id);
        match response {
            Ok(response) if control_plugins.is_instance_active(instance_id) => {
                let decision = match response.verdict {
                    ControlVerdict::Allow => EnforcementDecision::Allow,
                    ControlVerdict::Deny => EnforcementDecision::Deny,
                };
                if response.scope == DecisionScope::Reusable {
                    self.reusable_decisions
                        .insert(cache_key, CachedDecision { decision });
                }
                Ok(Decision {
                    decision,
                    rule: Some(rule),
                    source: DecisionSource::SyncPlugin {
                        instance_id: instance_id.to_string(),
                        timeout_ms,
                        concurrency_limit,
                        scope: response.scope,
                        reason: response.reason,
                    },
                })
            }
            Ok(_) => Ok(fallback_decision(
                rule,
                instance_id,
                timeout_ms,
                concurrency_limit,
                EnforcementDecision::Deny,
                SyncPluginFallbackReason::PluginError,
                Some("control plugin unloaded during file-policy decision".to_string()),
                None,
            )),
            Err(error) => Ok(fallback_decision(
                rule,
                instance_id,
                timeout_ms,
                concurrency_limit,
                fallback,
                SyncPluginFallbackReason::PluginError,
                Some(error.message),
                None,
            )),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn fallback_decision<'a>(
    rule: &'a EnforcementRule,
    instance_id: &str,
    timeout_ms: u64,
    concurrency_limit: u32,
    fallback: EnforcementDecision,
    reason: SyncPluginFallbackReason,
    error: Option<String>,
    in_flight: Option<u32>,
) -> Decision<'a> {
    Decision {
        decision: fallback,
        rule: Some(rule),
        source: DecisionSource::SyncPluginFallback {
            instance_id: instance_id.to_string(),
            timeout_ms,
            concurrency_limit,
            reason,
            error,
            in_flight,
            instance_concurrency_limit: None,
            instance_in_flight: None,
            fallback,
        },
    }
}

struct MutationRequest {
    pid: u32,
    operation: FilePolicyOperation,
    path_address: u64,
    dirfd: Option<i32>,
}

impl MutationRequest {
    fn from_notification(
        notification: &libc::seccomp_notif,
        configured: &BTreeSet<EnforcementSeccompSyscall>,
    ) -> Option<Self> {
        let Some(kernel_syscall) = KernelFileSeccompSyscall::from_seccomp(&notification.data)
        else {
            return None;
        };
        if !configured.contains(&kernel_syscall.configured()) {
            return None;
        }
        Some(Self {
            pid: notification.pid,
            operation: match kernel_syscall.configured() {
                EnforcementSeccompSyscall::Mkdir => FilePolicyOperation::Mkdir,
                EnforcementSeccompSyscall::Rmdir => FilePolicyOperation::Rmdir,
            },
            path_address: notification.data.args[kernel_syscall.path_argument()],
            dirfd: kernel_syscall
                .dirfd_argument()
                .map(|index| notification.data.args[index] as u32 as i32),
        })
    }

    fn absolute_path(&self, path_max_bytes: u32) -> Result<Option<PathBuf>, ControlError> {
        let path = read_c_string(self.pid, self.path_address, path_max_bytes)?;
        if path.truncated {
            return Err(ControlError::new(
                "file_enforcement_path",
                "seccomp path exceeded enforcement.seccomp_path_max_bytes",
            ));
        }
        let Some(raw_path) = path.value.filter(|path| !path.is_empty()) else {
            return Ok(None);
        };
        let raw = Path::new(&raw_path);
        let path = if raw.is_absolute() {
            raw.to_path_buf()
        } else {
            let base = if self.dirfd.is_none_or(|fd| fd == libc::AT_FDCWD) {
                PathBuf::from(format!("/proc/{}/cwd", self.pid))
            } else {
                PathBuf::from(format!(
                    "/proc/{}/fd/{}",
                    self.pid,
                    self.dirfd.expect("non-AT_FDCWD dirfd")
                ))
            };
            let base = match std::fs::read_link(base) {
                Ok(base) => base,
                Err(error) if target_exited(&error) => return Ok(None),
                Err(error) => {
                    return Err(ControlError::new(
                        "file_enforcement_path",
                        error.to_string(),
                    ));
                }
            };
            base.join(raw)
        };
        normalize_absolute_path(&path).map(Some)
    }
}

fn normalize_absolute_path(path: &Path) -> Result<PathBuf, ControlError> {
    if !path.is_absolute() {
        return Err(ControlError::new(
            "file_enforcement_path",
            format!("resolved path {} is not absolute", path.display()),
        ));
    }
    let mut normalized = PathBuf::from("/");
    for component in path.components() {
        match component {
            Component::RootDir | Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(part) => normalized.push(part),
            Component::Prefix(_) => {
                return Err(ControlError::new(
                    "file_enforcement_path",
                    "non-Unix path prefix",
                ));
            }
        }
    }
    Ok(normalized)
}

fn traced_process(
    trace_runtime: &TraceRuntime,
    process_registry: &ProcessIdentityManager,
    identity_reader: &impl ProcessIdentityReader,
    pid: u32,
) -> Result<Option<(TraceId, ProcessIdentity)>, ControlError> {
    Ok(TraceIdentityResolver::new(trace_runtime, process_registry)
        .read_and_match_pid(identity_reader, pid, "file_enforcement_identity")?
        .filter(|resolved| resolved.is_capturable())
        .map(|resolved| (resolved.trace_id, resolved.process)))
}
