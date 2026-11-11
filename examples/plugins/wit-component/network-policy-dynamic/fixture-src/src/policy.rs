//! Network policy configuration, publication and management commands.
mod commands;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use serde::{Deserialize, Serialize};
use spin::Mutex;

use crate::actrail::plugin::types::{
    ControlVerdict, NetworkPolicyApplyRequest, NetworkPolicyApplyStatus, NetworkPolicyDecision,
    NetworkPolicyMatchDryRunRequest, NetworkPolicyOperation, NetworkPolicyPatchItem,
    NetworkPolicyPatchOp, NetworkPolicyProtocol, NetworkPolicyRuleDraft,
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
    #[serde(default, skip_serializing_if = "PolicyOperation::is_default")]
    operation: PolicyOperation,
    #[serde(default, skip_serializing_if = "PolicyProtocol::is_default")]
    protocol: PolicyProtocol,
    endpoint: String,
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
            "dry-run" if argv.len() >= 3 => Self::dry_run(argv),
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
            if rule.endpoint.trim().is_empty() || rule.endpoint.contains('\0') {
                return Err(format!(
                    "rules[{index}].endpoint must be a non-empty endpoint selector"
                ));
            }
            if config.rules[..index].iter().any(|existing| {
                existing.endpoint == rule.endpoint
                    && existing.operation == rule.operation
                    && existing.protocol == rule.protocol
            }) {
                return Err(format!("duplicate endpoint selector {}", rule.endpoint));
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
                    operation: rule.operation.host_value(),
                    protocol: rule.protocol.host_value(),
                    endpoint: rule.endpoint.clone(),
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

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
enum PolicyOperation {
    #[default]
    Connect,
    Send,
    Bind,
    Listen,
}
impl PolicyOperation {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "connect" => Ok(Self::Connect),
            "send" => Ok(Self::Send),
            "bind" => Ok(Self::Bind),
            "listen" => Ok(Self::Listen),
            _ => Err(format!("invalid network operation {value}")),
        }
    }
    fn as_str(self) -> &'static str {
        match self {
            Self::Connect => "connect",
            Self::Send => "send",
            Self::Bind => "bind",
            Self::Listen => "listen",
        }
    }
    fn host_value(self) -> NetworkPolicyOperation {
        match self {
            Self::Connect => NetworkPolicyOperation::Connect,
            Self::Send => NetworkPolicyOperation::Send,
            Self::Bind => NetworkPolicyOperation::Bind,
            Self::Listen => NetworkPolicyOperation::Listen,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
enum PolicyProtocol {
    #[default]
    Any,
    Tcp,
    Udp,
}
impl PolicyProtocol {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "any" => Ok(Self::Any),
            "tcp" => Ok(Self::Tcp),
            "udp" => Ok(Self::Udp),
            _ => Err(format!("invalid network protocol {value}")),
        }
    }
    fn as_str(self) -> &'static str {
        match self {
            Self::Any => "any",
            Self::Tcp => "tcp",
            Self::Udp => "udp",
        }
    }
    fn host_value(self) -> NetworkPolicyProtocol {
        match self {
            Self::Any => NetworkPolicyProtocol::Any,
            Self::Tcp => NetworkPolicyProtocol::Tcp,
            Self::Udp => NetworkPolicyProtocol::Udp,
        }
    }
}
