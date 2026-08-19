use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use serde::{Deserialize, Serialize};
use spin::Mutex;

use crate::actrail::plugin::types::{
    ControlVerdict, NetworkPolicyApplyRequest, NetworkPolicyApplyStatus, NetworkPolicyDecision,
    NetworkPolicyMatchDryRunRequest, NetworkPolicyPatchItem, NetworkPolicyPatchOp,
    NetworkPolicyRuleDraft,
};

static POLICY_CONFIG: Mutex<Option<PolicyConfig>> = Mutex::new(None);
const GENERATED_ID_PREFIX: &str = "network-dynamic-";

pub(super) struct PolicyManager;

#[derive(Clone, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct PolicyConfig {
    #[serde(default)]
    rules: Vec<PolicyRule>,
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct PolicyRule {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rule_id: Option<String>,
    decision: PolicyDecision,
    remote: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    gray_target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    concurrency_limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fallback: Option<PolicyFallback>,
}

#[derive(Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
enum PolicyDecision {
    Allow,
    Deny,
    Gray,
}

#[derive(Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
enum PolicyFallback {
    Allow,
    Deny,
}

impl PolicyManager {
    pub(super) fn configuration_json() -> Result<String, String> {
        serde_json::to_string(&Self::current()?)
            .map_err(|error| format!("serialize network policy config: {error}"))
    }

    pub(super) fn validate_configuration(raw: &str) -> Result<Vec<String>, String> {
        let current = Self::current()?;
        let candidate = match Self::parse_and_normalize(raw, &current) {
            Ok(candidate) => candidate,
            Err(error) => return Ok(vec![error]),
        };
        if candidate == current {
            return Ok(Vec::new());
        }
        let request = Self::projection_request(&current, &candidate, "web config validation")?;
        let delete_count = current.rules.len() as u32;
        let result =
            crate::actrail::plugin::network_control_host::network_policy_rules_validate(&request)?;
        Ok(result
            .errors
            .into_iter()
            .map(|error| {
                if error.item_index >= delete_count {
                    format!(
                        "rule {}: {}",
                        error.item_index - delete_count,
                        error.message
                    )
                } else {
                    format!("existing rule {}: {}", error.item_index, error.message)
                }
            })
            .collect())
    }

    pub(super) fn submit_configuration(raw: &str) -> Result<(), String> {
        let current = POLICY_CONFIG.lock().clone().unwrap_or_default();
        let candidate = Self::parse_and_normalize(raw, &current)?;
        Self::publish_and_commit(current, candidate, "plugin config submit")
    }

    pub(super) fn handle_command(argv: &[String]) -> Result<String, String> {
        if argv.len() == 1 && (argv[0] == "help" || argv[0] == "--help") {
            return Ok(Self::help());
        }
        if argv.len() < 2 || argv[0] != "rule" {
            return Err(Self::usage());
        }
        match argv[1].as_str() {
            "list" if argv.len() == 2 => Self::list_rules(),
            "dry-run" if argv.len() == 3 => Self::dry_run(&argv[2]),
            "upsert" => Self::upsert(argv),
            "delete" if argv.len() == 3 => Self::delete(&argv[2]),
            _ => Err(Self::usage()),
        }
    }

    fn current() -> Result<PolicyConfig, String> {
        POLICY_CONFIG
            .lock()
            .clone()
            .ok_or_else(|| "plugin runtime config has not been initialized".to_string())
    }

    fn parse_and_normalize(raw: &str, current: &PolicyConfig) -> Result<PolicyConfig, String> {
        let mut config = serde_json::from_str::<PolicyConfig>(raw)
            .map_err(|error| format!("parse config JSON: {error}"))?;
        Self::normalize_rule_ids(&mut config, current)?;
        Self::validate_config(&config)?;
        Ok(config)
    }

    fn normalize_rule_ids(config: &mut PolicyConfig, current: &PolicyConfig) -> Result<(), String> {
        let mut next_id = 1_u64;
        for rule in current.rules.iter().chain(config.rules.iter()) {
            if let Some(number) = rule
                .rule_id
                .as_deref()
                .and_then(|id| id.strip_prefix(GENERATED_ID_PREFIX))
                .and_then(|number| number.parse::<u64>().ok())
            {
                next_id = next_id.max(
                    number
                        .checked_add(1)
                        .ok_or_else(|| "generated network policy rule id overflow".to_string())?,
                );
            }
        }
        for rule in &mut config.rules {
            if rule
                .rule_id
                .as_deref()
                .is_none_or(|id| id.trim().is_empty())
            {
                rule.rule_id = Some(format!("{GENERATED_ID_PREFIX}{next_id}"));
                next_id = next_id
                    .checked_add(1)
                    .ok_or_else(|| "generated network policy rule id overflow".to_string())?;
            }
        }
        Ok(())
    }

    fn validate_config(config: &PolicyConfig) -> Result<(), String> {
        for (index, rule) in config.rules.iter().enumerate() {
            let id = rule
                .rule_id
                .as_deref()
                .ok_or_else(|| format!("rules[{index}].rule_id is required"))?;
            if id.trim().is_empty() || id.chars().any(char::is_whitespace) {
                return Err(format!(
                    "rules[{index}].rule_id must be non-empty and contain no whitespace"
                ));
            }
            if config.rules[..index]
                .iter()
                .any(|existing| existing.rule_id.as_deref() == Some(id))
            {
                return Err(format!("duplicate rule_id {id}"));
            }
            if rule.remote.trim().is_empty() || rule.remote.contains('\0') {
                return Err(format!(
                    "rules[{index}].remote must be a non-empty remote selector"
                ));
            }
            if config.rules[..index]
                .iter()
                .any(|existing| existing.remote == rule.remote)
            {
                return Err(format!("duplicate remote selector {}", rule.remote));
            }
            Self::validate_rule_shape(rule, index)?;
        }
        Ok(())
    }

    fn validate_rule_shape(rule: &PolicyRule, index: usize) -> Result<(), String> {
        let has_gray_fields = rule.gray_target.is_some()
            || rule.timeout_ms.is_some()
            || rule.concurrency_limit.is_some()
            || rule.fallback.is_some();
        match rule.decision {
            PolicyDecision::Allow | PolicyDecision::Deny if has_gray_fields => Err(format!(
                "rules[{index}] gray settings are only valid when decision is gray"
            )),
            PolicyDecision::Gray => {
                if rule
                    .gray_target
                    .as_deref()
                    .is_none_or(|target| target.trim().is_empty())
                {
                    return Err(format!("rules[{index}].gray_target is required for gray"));
                }
                if rule.timeout_ms.is_none_or(|value| value == 0) {
                    return Err(format!("rules[{index}].timeout_ms must be positive"));
                }
                if rule.concurrency_limit.is_none_or(|value| value == 0) {
                    return Err(format!("rules[{index}].concurrency_limit must be positive"));
                }
                if rule.fallback.is_none() {
                    return Err(format!("rules[{index}].fallback is required for gray"));
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn projection_request(
        current: &PolicyConfig,
        candidate: &PolicyConfig,
        reason: &str,
    ) -> Result<NetworkPolicyApplyRequest, String> {
        let revision =
            crate::actrail::plugin::network_control_host::network_policy_rules_version_get()?;
        let mut items = Vec::with_capacity(current.rules.len() + candidate.rules.len());
        for rule in &current.rules {
            items.push(NetworkPolicyPatchItem {
                op: NetworkPolicyPatchOp::Delete,
                rule_id: rule.rule_id.clone(),
                rule: None,
            });
        }
        for rule in &candidate.rules {
            items.push(NetworkPolicyPatchItem {
                op: NetworkPolicyPatchOp::Upsert,
                rule_id: rule.rule_id.clone(),
                rule: Some(NetworkPolicyRuleDraft {
                    rule_id: rule.rule_id.clone(),
                    decision: rule.decision.host_value(),
                    remote: rule.remote.clone(),
                    gray_target: rule.gray_target.clone(),
                    timeout_ms: rule.timeout_ms,
                    concurrency_limit: rule.concurrency_limit,
                    fallback: rule.fallback.map(PolicyFallback::host_value),
                }),
            });
        }
        Ok(NetworkPolicyApplyRequest {
            base_revision: revision,
            mutation_id: "network-dynamic-policy-config".to_string(),
            reason: Some(reason.to_string()),
            items,
        })
    }

    fn publish_and_commit(
        current: PolicyConfig,
        candidate: PolicyConfig,
        reason: &str,
    ) -> Result<(), String> {
        if current == candidate {
            let mut stored = POLICY_CONFIG.lock();
            if stored.is_none() {
                crate::actrail::plugin::network_control_host::network_policy_rules_version_get()?;
                *stored = Some(candidate);
            }
            return Ok(());
        }
        let request = Self::projection_request(&current, &candidate, reason)?;
        let expected = request.items.len() as u32;
        let validation =
            crate::actrail::plugin::network_control_host::network_policy_rules_validate(&request)?;
        if !matches!(validation.status, NetworkPolicyApplyStatus::Accepted) {
            return Err(Self::first_apply_error(
                &validation.errors,
                "validation rejected",
            ));
        }
        let result =
            crate::actrail::plugin::network_control_host::network_policy_rules_apply(&request)?;
        if !matches!(result.status, NetworkPolicyApplyStatus::Accepted)
            || result.applied_count != expected
        {
            return Err(Self::first_apply_error(&result.errors, "apply rejected"));
        }
        *POLICY_CONFIG.lock() = Some(candidate);
        Ok(())
    }

    fn first_apply_error(
        errors: &[crate::actrail::plugin::types::NetworkPolicyApplyError],
        fallback: &str,
    ) -> String {
        errors
            .first()
            .map(|error| error.message.clone())
            .unwrap_or_else(|| fallback.to_string())
    }

    fn upsert(argv: &[String]) -> Result<String, String> {
        if argv.len() < 4 {
            return Err(Self::usage());
        }
        let decision = PolicyDecision::parse(&argv[2])?;
        let remote = argv[3].clone();
        let mut replacement = PolicyRule {
            rule_id: None,
            decision,
            remote: remote.clone(),
            gray_target: None,
            timeout_ms: None,
            concurrency_limit: None,
            fallback: None,
        };
        let mut index = 4;
        while index < argv.len() {
            let value = argv
                .get(index + 1)
                .ok_or_else(|| format!("{} requires a value", argv[index]))?;
            match argv[index].as_str() {
                "--rule-id" => replacement.rule_id = Some(value.clone()),
                "--gray-target" => replacement.gray_target = Some(value.clone()),
                "--timeout-ms" => {
                    replacement.timeout_ms = Some(
                        value
                            .parse::<u64>()
                            .map_err(|_| "invalid --timeout-ms value".to_string())?,
                    )
                }
                "--concurrency" => {
                    replacement.concurrency_limit = Some(
                        value
                            .parse::<u32>()
                            .map_err(|_| "invalid --concurrency value".to_string())?,
                    )
                }
                "--fallback" => replacement.fallback = Some(PolicyFallback::parse(value)?),
                _ => return Err(Self::usage()),
            }
            index += 2;
        }
        let current = Self::current()?;
        let mut candidate = current.clone();
        if let Some(rule_id) = replacement.rule_id.as_deref()
            && let Some(existing) = candidate
                .rules
                .iter_mut()
                .find(|rule| rule.rule_id.as_deref() == Some(rule_id))
        {
            *existing = replacement;
        } else {
            candidate.rules.push(replacement);
        }
        Self::normalize_rule_ids(&mut candidate, &current)?;
        Self::validate_config(&candidate)?;
        let rule_id = candidate
            .rules
            .iter()
            .find(|rule| rule.remote == remote)
            .and_then(|rule| rule.rule_id.clone())
            .ok_or_else(|| "normalized network rule has no id".to_string())?;
        Self::publish_and_commit(current, candidate, "dynamic plugin command")?;
        Ok(format!("accepted rule_id={rule_id} remote={remote}\n"))
    }

    fn delete(rule_id: &str) -> Result<String, String> {
        let current = Self::current()?;
        let mut candidate = current.clone();
        let before = candidate.rules.len();
        candidate
            .rules
            .retain(|rule| rule.rule_id.as_deref() != Some(rule_id));
        if candidate.rules.len() == before {
            return Err(format!("rule {rule_id} not found"));
        }
        Self::publish_and_commit(current, candidate, "dynamic plugin command")?;
        Ok(format!("accepted deleted={rule_id}\n"))
    }

    fn list_rules() -> Result<String, String> {
        let mut output = String::new();
        for rule in Self::current()?.rules {
            output.push_str(rule.rule_id.as_deref().unwrap_or("unassigned"));
            output.push(' ');
            output.push_str(rule.decision.as_str());
            output.push(' ');
            output.push_str(&rule.remote);
            if let Some(target) = rule.gray_target {
                output.push_str(" gray_target=");
                output.push_str(&target);
            }
            output.push('\n');
        }
        if output.is_empty() {
            output.push_str("no configured rules\n");
        }
        Ok(output)
    }

    fn dry_run(remote: &str) -> Result<String, String> {
        let result =
            crate::actrail::plugin::network_control_host::network_policy_rules_match_dry_run(
                &NetworkPolicyMatchDryRunRequest {
                    remote: remote.to_string(),
                },
            )?;
        Ok(format!(
            "matched={} decision={} rule_id={} owner={} remote={} rule_revision={} source_revision={}\n",
            result.matched,
            Self::host_decision_name(result.decision),
            result.rule_id.unwrap_or_else(|| "none".to_string()),
            result
                .owner_instance_id
                .unwrap_or_else(|| "none".to_string()),
            result.resolved_remote,
            result
                .rule_revision
                .map(|value| value.to_string())
                .unwrap_or_else(|| "none".to_string()),
            result.source_revision,
        ))
    }

    fn host_decision_name(decision: NetworkPolicyDecision) -> &'static str {
        match decision {
            NetworkPolicyDecision::Default => "default",
            NetworkPolicyDecision::Allow => "allow",
            NetworkPolicyDecision::Deny => "deny",
            NetworkPolicyDecision::Gray => "gray",
        }
    }

    fn usage() -> String {
        "usage: help | rule list | rule dry-run <ip:port> | rule upsert <allow|deny|gray> <ip:port|ip:*> [--rule-id ID] [--gray-target INSTANCE --timeout-ms N --concurrency N --fallback allow|deny] | rule delete <rule-id>".to_string()
    }

    fn help() -> String {
        "supported commands:\n  help\n  rule list\n  rule dry-run <ip:port>\n  rule upsert <allow|deny|gray> <ip:port|ip:*> [--rule-id ID] [--gray-target INSTANCE --timeout-ms N --concurrency N --fallback allow|deny]\n  rule delete <rule-id>\n".to_string()
    }
}

impl PolicyDecision {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "allow" => Ok(Self::Allow),
            "deny" => Ok(Self::Deny),
            "gray" => Ok(Self::Gray),
            _ => Err("decision must be allow, deny, or gray".to_string()),
        }
    }

    fn host_value(self) -> NetworkPolicyDecision {
        match self {
            Self::Allow => NetworkPolicyDecision::Allow,
            Self::Deny => NetworkPolicyDecision::Deny,
            Self::Gray => NetworkPolicyDecision::Gray,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Deny => "deny",
            Self::Gray => "gray",
        }
    }
}

impl PolicyFallback {
    fn parse(value: &str) -> Result<Self, String> {
        match value.as_ref() {
            "allow" => Ok(Self::Allow),
            "deny" => Ok(Self::Deny),
            _ => Err("fallback must be allow or deny".to_string()),
        }
    }

    fn host_value(self) -> ControlVerdict {
        match self {
            Self::Allow => ControlVerdict::Allow,
            Self::Deny => ControlVerdict::Deny,
        }
    }
}
