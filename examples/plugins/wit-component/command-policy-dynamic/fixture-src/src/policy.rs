use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use serde::{Deserialize, Serialize};
use spin::Mutex;

use crate::actrail::plugin::types::{
    CommandPolicyApplyRequest, CommandPolicyApplyStatus, CommandPolicyDecision,
    CommandPolicyMatchDryRunRequest, CommandPolicyPatchItem, CommandPolicyPatchOp,
    CommandPolicyRuleDraft,
};

static POLICY_CONFIG: Mutex<Option<PolicyConfig>> = Mutex::new(None);
const GENERATED_ID_PREFIX: &str = "command-dynamic-";

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
    executable: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    args: Option<Vec<String>>,
    #[serde(default = "PolicyManager::default_priority")]
    priority: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    gray_target: Option<String>,
}

#[derive(Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
enum PolicyDecision {
    Allow,
    Deny,
    Gray,
}

impl PolicyManager {
    pub(super) fn configuration_json() -> Result<String, String> {
        serde_json::to_string(&Self::current()?)
            .map_err(|error| format!("serialize command policy config: {error}"))
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
        let result = crate::actrail::plugin::host::command_policy_rules_validate(&request)?;
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
            "dry-run" => Self::dry_run(argv),
            "upsert" => Self::upsert(argv),
            "delete" => Self::delete(argv),
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
        Self::validate_config(&mut config)?;
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
                        .ok_or_else(|| "generated command policy rule id overflow".to_string())?,
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
                    .ok_or_else(|| "generated command policy rule id overflow".to_string())?;
            }
        }
        Ok(())
    }

    fn validate_config(config: &mut PolicyConfig) -> Result<(), String> {
        for index in 0..config.rules.len() {
            let rule = &config.rules[index];
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
            match rule.decision {
                PolicyDecision::Gray
                    if rule
                        .gray_target
                        .as_deref()
                        .is_none_or(|target| target.trim().is_empty()) =>
                {
                    return Err(format!("rules[{index}].gray_target is required for gray"));
                }
                PolicyDecision::Allow | PolicyDecision::Deny if rule.gray_target.is_some() => {
                    return Err(format!("rules[{index}].gray_target is only valid for gray"));
                }
                _ => {}
            }
            Self::validate_args(rule.args.as_deref(), index)?;
        }
        for rule in &mut config.rules {
            rule.executable = Self::normalize_absolute(&rule.executable)?;
        }
        for index in 0..config.rules.len() {
            if config.rules[..index]
                .iter()
                .any(|existing| {
                    existing.executable == config.rules[index].executable
                        && Self::same_args_scope(
                            existing.args.as_deref(),
                            config.rules[index].args.as_deref(),
                        )
                })
            {
                return Err(format!(
                    "duplicate executable {} args scope",
                    config.rules[index].executable,
                ));
            }
        }
        Ok(())
    }

    fn normalize_absolute(raw: &str) -> Result<String, String> {
        if !raw.starts_with('/') {
            return Err(format!("command executable {raw} must be absolute"));
        }
        let mut parts = Vec::new();
        for part in raw.split('/') {
            match part {
                "" | "." => {}
                ".." => {
                    parts.pop();
                }
                value => parts.push(value),
            }
        }
        let mut normalized = String::from("/");
        normalized.push_str(&parts.join("/"));
        Ok(normalized)
    }

    fn validate_args(args: Option<&[String]>, rule_index: usize) -> Result<(), String> {
        let Some(args) = args else {
            return Ok(());
        };
        for (index, arg) in args.iter().enumerate() {
            if arg.contains('\0') {
                return Err(format!("rules[{rule_index}].args[{index}] contains NUL"));
            }
            if arg == "*" && index + 1 != args.len() {
                return Err(format!(
                    "rules[{rule_index}].args wildcard * is only allowed as the final item"
                ));
            }
        }
        Ok(())
    }

    fn same_args_scope(left: Option<&[String]>, right: Option<&[String]>) -> bool {
        Self::args_scope(left) == Self::args_scope(right)
    }

    fn args_scope(args: Option<&[String]>) -> (u8, &[String]) {
        match args {
            None => (0, &[]),
            Some([wildcard]) if wildcard == "*" => (0, &[]),
            Some(args) if args.last().is_some_and(|arg| arg == "*") => {
                (2, &args[..args.len() - 1])
            }
            Some(args) => (1, args),
        }
    }

    fn projection_request(
        current: &PolicyConfig,
        candidate: &PolicyConfig,
        reason: &str,
    ) -> Result<CommandPolicyApplyRequest, String> {
        let revision = crate::actrail::plugin::host::command_policy_rules_version_get()?;
        let mut items = Vec::with_capacity(current.rules.len() + candidate.rules.len());
        for rule in &current.rules {
            items.push(CommandPolicyPatchItem {
                op: CommandPolicyPatchOp::Delete,
                rule_id: rule.rule_id.clone(),
                rule: None,
            });
        }
        for rule in &candidate.rules {
            items.push(CommandPolicyPatchItem {
                op: CommandPolicyPatchOp::Upsert,
                rule_id: rule.rule_id.clone(),
                rule: Some(CommandPolicyRuleDraft {
                    rule_id: rule.rule_id.clone(),
                    decision: rule.decision.host_value(),
                    executable: rule.executable.clone(),
                    args: rule.args.clone(),
                    gray_target: rule.gray_target.clone(),
                    priority: rule.priority,
                }),
            });
        }
        Ok(CommandPolicyApplyRequest {
            base_revision: revision,
            mutation_id: "command-dynamic-policy-config".to_string(),
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
                *stored = Some(candidate);
            }
            return Ok(());
        }
        let request = Self::projection_request(&current, &candidate, reason)?;
        let expected = request.items.len() as u32;
        let validation = crate::actrail::plugin::host::command_policy_rules_validate(&request)?;
        if !matches!(validation.status, CommandPolicyApplyStatus::Accepted) {
            return Err(Self::first_apply_error(
                &validation.errors,
                "validation rejected",
            ));
        }
        let result = crate::actrail::plugin::host::command_policy_rules_apply(&request)?;
        if !matches!(result.status, CommandPolicyApplyStatus::Accepted)
            || result.applied_count != expected
        {
            return Err(Self::first_apply_error(&result.errors, "apply rejected"));
        }
        *POLICY_CONFIG.lock() = Some(candidate);
        Ok(())
    }

    fn first_apply_error(
        errors: &[crate::actrail::plugin::types::CommandPolicyApplyError],
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
        let executable = argv[3].clone();
        let normalized_executable = Self::normalize_absolute(&executable)?;
        let mut priority = Self::default_priority();
        let mut gray_target = None;
        let mut args = None;
        let mut rule_id = None;
        let mut index = 4;
        while index < argv.len() {
            let value = argv
                .get(index + 1)
                .ok_or_else(|| format!("{} requires a value", argv[index]))?;
            match argv[index].as_str() {
                "--priority" => {
                    priority = value
                        .parse::<i32>()
                        .map_err(|_| "invalid --priority i32 value".to_string())?;
                }
                "--gray-target" => gray_target = Some(value.clone()),
                "--args-json" => {
                    args = Some(
                        serde_json::from_str::<Vec<String>>(value).map_err(|error| {
                            format!("--args-json must be a JSON string array: {error}")
                        })?,
                    )
                }
                "--rule-id" => rule_id = Some(value.clone()),
                _ => return Err(Self::usage()),
            }
            index += 2;
        }
        let current = Self::current()?;
        let mut candidate = current.clone();
        let replacement = PolicyRule {
            rule_id: rule_id.clone(),
            decision,
            executable: executable.clone(),
            args,
            priority,
            gray_target,
        };
        if let Some(rule_id) = rule_id.as_deref() {
            if let Some(existing) = candidate
                .rules
                .iter_mut()
                .find(|rule| rule.rule_id.as_deref() == Some(rule_id))
            {
                *existing = replacement;
            } else {
                candidate.rules.push(replacement);
            }
        } else {
            candidate.rules.push(replacement);
        }
        Self::normalize_rule_ids(&mut candidate, &current)?;
        Self::validate_config(&mut candidate)?;
        let rule_id = match rule_id.as_deref() {
            Some(rule_id) => candidate
                .rules
                .iter()
                .find(|rule| rule.rule_id.as_deref() == Some(rule_id)),
            None => candidate.rules.last(),
        }
        .filter(|rule| rule.executable == normalized_executable)
        .and_then(|rule| rule.rule_id.clone())
        .ok_or_else(|| "normalized command rule has no id".to_string())?;
        Self::publish_and_commit(current, candidate, "dynamic plugin command")?;
        Ok(format!(
            "accepted rule_id={rule_id} executable={executable}\n"
        ))
    }

    fn delete(argv: &[String]) -> Result<String, String> {
        if argv.len() != 3 {
            return Err(Self::usage());
        }
        let current = Self::current()?;
        let mut candidate = current.clone();
        let before = candidate.rules.len();
        candidate
            .rules
            .retain(|rule| rule.rule_id.as_deref() != Some(argv[2].as_str()));
        if candidate.rules.len() == before {
            return Err(format!("rule {} not found", argv[2]));
        }
        Self::publish_and_commit(current, candidate, "dynamic plugin command")?;
        Ok(format!("accepted deleted={}\n", argv[2]))
    }

    fn list_rules() -> Result<String, String> {
        let mut output = String::new();
        for rule in Self::current()?.rules {
            output.push_str(rule.rule_id.as_deref().unwrap_or("unassigned"));
            output.push(' ');
            output.push_str(rule.decision.as_str());
            output.push(' ');
            output.push_str(&rule.executable);
            if let Some(args) = rule.args {
                output.push_str(" args_json=");
                output.push_str(
                    &serde_json::to_string(&args)
                        .map_err(|error| format!("serialize command args: {error}"))?,
                );
            }
            output.push_str(" priority=");
            output.push_str(&rule.priority.to_string());
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

    fn dry_run(argv: &[String]) -> Result<String, String> {
        if argv.len() != 3 && argv.len() != 5 {
            return Err(Self::usage());
        }
        let args = if argv.len() == 5 && argv[3] == "--args-json" {
            serde_json::from_str::<Vec<String>>(&argv[4])
                .map_err(|error| format!("--args-json must be a JSON string array: {error}"))?
        } else if argv.len() == 3 {
            Vec::new()
        } else {
            return Err(Self::usage());
        };
        let result = crate::actrail::plugin::host::command_policy_rules_match_dry_run(
            &CommandPolicyMatchDryRunRequest {
                executable: argv[2].clone(),
                args,
            },
        )?;
        Ok(format!(
            "matched={} decision={} rule_id={} owner={} executable={} rule_revision={} source_revision={}\n",
            result.matched,
            Self::host_decision_name(result.decision),
            result.rule_id.unwrap_or_else(|| "none".to_string()),
            result
                .owner_instance_id
                .unwrap_or_else(|| "none".to_string()),
            result.resolved_executable,
            result
                .rule_revision
                .map(|value| value.to_string())
                .unwrap_or_else(|| "none".to_string()),
            result.source_revision,
        ))
    }

    fn host_decision_name(decision: CommandPolicyDecision) -> &'static str {
        match decision {
            CommandPolicyDecision::Default => "default",
            CommandPolicyDecision::Allow => "allow",
            CommandPolicyDecision::Deny => "deny",
            CommandPolicyDecision::Gray => "gray",
        }
    }

    fn default_priority() -> i32 {
        10
    }

    fn usage() -> String {
        "usage: help | rule list | rule dry-run <absolute-executable> [--args-json JSON] | rule upsert <allow|deny|gray> <absolute-executable> [--args-json JSON] [--priority N] [--gray-target INSTANCE-ID] [--rule-id ID] | rule delete <rule-id>".to_string()
    }

    fn help() -> String {
        "supported commands:\n  help\n  rule list\n  rule dry-run <absolute-executable> [--args-json JSON]\n  rule upsert <allow|deny|gray> <absolute-executable> [--args-json JSON] [--priority N] [--gray-target INSTANCE-ID] [--rule-id ID]\n  rule delete <rule-id>\n".to_string()
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

    fn host_value(self) -> CommandPolicyDecision {
        match self {
            Self::Allow => CommandPolicyDecision::Allow,
            Self::Deny => CommandPolicyDecision::Deny,
            Self::Gray => CommandPolicyDecision::Gray,
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
