//! Command-execution control policy and dispatch helpers.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use config_core::daemon::{CommandControlConfig, EnforcementDecision};
use control_contract::reply::ControlError;
use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;
use plugin_system::{
    CONTROL_CURRENT_CONTEXT_TOKEN, ControlDecisionBudget, ControlDecisionRequest, ControlSubject,
    ControlVerdict,
};
use process_identity::ProcessIdentityManager;

use crate::services::control_runtime::ControlPluginRuntime;
use crate::services::identity::ControlActorIdentityResolver;
use crate::services::process_seccomp::ProcessSeccompExecCandidate;

#[derive(Debug)]
pub(crate) struct CommandControlService {
    enabled: bool,
    rules: CommandControlRules,
    in_flight_by_rule: Mutex<BTreeMap<String, u32>>,
}

impl CommandControlService {
    pub(crate) fn new(config: &CommandControlConfig) -> Result<Self, ControlError> {
        let rules = if config.enabled {
            CommandControlRules::load(&config.rules_path)?
        } else {
            CommandControlRules::default()
        };
        Ok(Self {
            enabled: config.enabled,
            rules,
            in_flight_by_rule: Mutex::new(BTreeMap::new()),
        })
    }

    pub(in crate::services) fn decide_exec(
        &self,
        trace_id: TraceId,
        process: &ProcessIdentity,
        process_registry: &ProcessIdentityManager,
        candidate: &ProcessSeccompExecCandidate,
        control_plugins: &ControlPluginRuntime,
    ) -> Result<CommandControlOutcome, ControlError> {
        if !self.enabled || candidate.path_truncated {
            return Ok(CommandControlOutcome::Continue);
        }
        let Some(path) = candidate.path.as_deref() else {
            return Ok(CommandControlOutcome::Continue);
        };
        let Some(rule) = self.rules.find_path(Path::new(path)) else {
            return Ok(CommandControlOutcome::Continue);
        };
        if !self.try_reserve_rule(rule)? {
            return Ok(CommandControlOutcome::DecisionError {
                decision: rule.fallback,
                rule_id: rule.rule_id.clone(),
                plugin_instance: rule.instance_id.clone(),
                timeout_ms: rule.timeout_ms,
                concurrency_limit: rule.concurrency_limit,
                error: "concurrency_limit".to_string(),
            });
        }
        let result = self.decide_rule(
            trace_id,
            process,
            process_registry,
            candidate,
            control_plugins,
            rule,
        );
        self.release_rule(&rule.rule_id);
        result
    }

    fn decide_rule(
        &self,
        trace_id: TraceId,
        process: &ProcessIdentity,
        process_registry: &ProcessIdentityManager,
        candidate: &ProcessSeccompExecCandidate,
        control_plugins: &ControlPluginRuntime,
        rule: &CommandControlRule,
    ) -> Result<CommandControlOutcome, ControlError> {
        let request = ControlDecisionRequest {
            decision_id: format!("{}:{trace_id}", rule.rule_id),
            trace_id: trace_id.to_string(),
            subject: ControlSubject::CommandExecution,
            actor_process_identity: ControlActorIdentityResolver::new(process_registry)
                .resolve(*process)?,
            operation: candidate.syscall.clone(),
            target_summary: command_target_summary(candidate),
            context_ref: Some(CONTROL_CURRENT_CONTEXT_TOKEN.to_string()),
            file_policy_context: None,
        };
        let response = control_plugins
            .decide(
                &rule.instance_id,
                request,
                ControlDecisionBudget {
                    timeout_ms: Some(rule.timeout_ms),
                },
            )
            .map_err(|error| {
                ControlError::new(
                    "command_control_plugin",
                    format!("{}: {}", error.code, error.message),
                )
            });
        match response {
            Ok(response) => {
                let decision = decision_to_enforcement(response.verdict);
                Ok(CommandControlOutcome::Decision {
                    decision,
                    rule_id: rule.rule_id.clone(),
                    plugin_instance: rule.instance_id.clone(),
                    timeout_ms: rule.timeout_ms,
                    concurrency_limit: rule.concurrency_limit,
                })
            }
            Err(error) => Ok(CommandControlOutcome::Decision {
                decision: rule.fallback,
                rule_id: rule.rule_id.clone(),
                plugin_instance: rule.instance_id.clone(),
                timeout_ms: rule.timeout_ms,
                concurrency_limit: rule.concurrency_limit,
            }
            .with_error(error.message)),
        }
    }

    fn try_reserve_rule(&self, rule: &CommandControlRule) -> Result<bool, ControlError> {
        let mut in_flight = self.in_flight_by_rule.lock().map_err(|error| {
            ControlError::new(
                "command_control_policy",
                format!("in-flight lock poisoned: {error}"),
            )
        })?;
        let current = in_flight.get(&rule.rule_id).copied().unwrap_or_default();
        if current >= rule.concurrency_limit {
            return Ok(false);
        }
        in_flight.insert(rule.rule_id.clone(), current + 1);
        Ok(true)
    }

    fn release_rule(&self, rule_id: &str) {
        let Ok(mut in_flight) = self.in_flight_by_rule.lock() else {
            return;
        };
        match in_flight.get_mut(rule_id) {
            Some(count) if *count > 1 => *count -= 1,
            Some(_) => {
                in_flight.remove(rule_id);
            }
            None => {}
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CommandControlOutcome {
    Continue,
    Decision {
        decision: EnforcementDecision,
        rule_id: String,
        plugin_instance: String,
        timeout_ms: u64,
        concurrency_limit: u32,
    },
    DecisionError {
        decision: EnforcementDecision,
        rule_id: String,
        plugin_instance: String,
        timeout_ms: u64,
        concurrency_limit: u32,
        error: String,
    },
}

impl CommandControlOutcome {
    fn with_error(self, error: String) -> Self {
        match self {
            Self::Decision {
                decision,
                rule_id,
                plugin_instance,
                timeout_ms,
                concurrency_limit,
            } => Self::DecisionError {
                decision,
                rule_id,
                plugin_instance,
                timeout_ms,
                concurrency_limit,
                error,
            },
            other => other,
        }
    }
}

#[derive(Debug, Default)]
struct CommandControlRules {
    by_path: BTreeMap<PathBuf, CommandControlRule>,
}

impl CommandControlRules {
    fn load(path: &Path) -> Result<Self, ControlError> {
        let raw = std::fs::read_to_string(path).map_err(|error| {
            ControlError::new(
                "command_control_policy",
                format!(
                    "read command control rules {} failed: {error}",
                    path.display()
                ),
            )
        })?;
        let mut rules = Self::default();
        for (index, raw_line) in raw.lines().enumerate() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let rule = parse_rule(line).map_err(|message| {
                ControlError::new(
                    "command_control_policy",
                    format!("{}:{}: {message}", path.display(), index + 1),
                )
            })?;
            if rules.by_path.insert(rule.path.clone(), rule).is_some() {
                return Err(ControlError::new(
                    "command_control_policy",
                    format!("{}:{}: duplicate command path", path.display(), index + 1),
                ));
            }
        }
        Ok(rules)
    }

    fn find_path(&self, path: &Path) -> Option<&CommandControlRule> {
        self.by_path.get(path)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CommandControlRule {
    rule_id: String,
    instance_id: String,
    timeout_ms: u64,
    concurrency_limit: u32,
    fallback: EnforcementDecision,
    path: PathBuf,
}

fn parse_rule(line: &str) -> Result<CommandControlRule, String> {
    let parts = line.split_whitespace().collect::<Vec<_>>();
    if parts.len() != 11 {
        return Err("expected: <rule_id> sync-plugin <instance> timeout-ms <positive-ms> concurrency <positive-limit> fallback <allow|deny> exec <absolute-path>".to_string());
    }
    if parts[1] != "sync-plugin"
        || parts[3] != "timeout-ms"
        || parts[5] != "concurrency"
        || parts[7] != "fallback"
        || parts[9] != "exec"
    {
        return Err("expected: <rule_id> sync-plugin <instance> timeout-ms <positive-ms> concurrency <positive-limit> fallback <allow|deny> exec <absolute-path>".to_string());
    }
    let timeout_ms = positive_u64("timeout-ms", parts[4])?;
    let concurrency_limit = positive_u32("concurrency", parts[6])?;
    let fallback = parse_decision(parts[8])?;
    let path = PathBuf::from(parts[10]);
    if !path.is_absolute() {
        return Err("command rule exec path must be absolute".to_string());
    }
    Ok(CommandControlRule {
        rule_id: parts[0].to_string(),
        instance_id: parts[2].to_string(),
        timeout_ms,
        concurrency_limit,
        fallback,
        path,
    })
}

fn parse_decision(value: &str) -> Result<EnforcementDecision, String> {
    match value {
        "allow" => Ok(EnforcementDecision::Allow),
        "deny" => Ok(EnforcementDecision::Deny),
        _ => Err("fallback must be allow or deny".to_string()),
    }
}

fn decision_to_enforcement(verdict: ControlVerdict) -> EnforcementDecision {
    match verdict {
        ControlVerdict::Allow => EnforcementDecision::Allow,
        ControlVerdict::Deny => EnforcementDecision::Deny,
    }
}

fn positive_u64(label: &str, value: &str) -> Result<u64, String> {
    let parsed = value
        .parse::<u64>()
        .map_err(|error| format!("{label} must be a positive integer: {error}"))?;
    if parsed == 0 {
        return Err(format!("{label} must be greater than zero"));
    }
    Ok(parsed)
}

fn positive_u32(label: &str, value: &str) -> Result<u32, String> {
    let parsed = value
        .parse::<u32>()
        .map_err(|error| format!("{label} must be a positive integer: {error}"))?;
    if parsed == 0 {
        return Err(format!("{label} must be greater than zero"));
    }
    Ok(parsed)
}

fn command_target_summary(candidate: &ProcessSeccompExecCandidate) -> String {
    let argv = if candidate.argv.is_empty() {
        String::new()
    } else {
        candidate.argv.join(" ")
    };
    format!(
        "path={} argv={}",
        candidate.path.as_deref().unwrap_or_default(),
        argv
    )
}
