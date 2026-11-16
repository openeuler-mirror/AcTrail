//! Network policy management commands.
use super::*;

impl PolicyManager {
    pub(super) fn upsert(argv: &[String]) -> Result<String, String> {
        if argv.len() < 4 {
            return Err(Self::usage());
        }
        let decision = PolicyDecision::parse(&argv[2])?;
        let endpoint = argv[3].clone();
        let mut replacement = PolicyRule {
            rule_id: None,
            decision,
            operation: PolicyOperation::default(),
            protocol: PolicyProtocol::default(),
            endpoint: endpoint.clone(),
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
                "--operation" => replacement.operation = PolicyOperation::parse(value)?,
                "--protocol" => replacement.protocol = PolicyProtocol::parse(value)?,
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
        let operation = replacement.operation;
        let protocol = replacement.protocol;
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
            .find(|rule| {
                rule.endpoint == endpoint
                    && rule.operation == operation
                    && rule.protocol == protocol
            })
            .and_then(|rule| rule.rule_id.clone())
            .ok_or_else(|| "normalized network rule has no id".to_string())?;
        Self::publish_and_commit(current, candidate, "dynamic plugin command")?;
        Ok(format!("accepted rule_id={rule_id} endpoint={endpoint}\n"))
    }

    pub(super) fn delete(rule_id: &str) -> Result<String, String> {
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

    pub(super) fn list_rules() -> Result<String, String> {
        let mut output = String::new();
        for rule in Self::current()?.rules {
            output.push_str(rule.rule_id.as_deref().unwrap_or("unassigned"));
            output.push(' ');
            output.push_str(rule.decision.as_str());
            output.push(' ');
            output.push_str(rule.operation.as_str());
            output.push(' ');
            output.push_str(rule.protocol.as_str());
            output.push(' ');
            output.push_str(&rule.endpoint);
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

    pub(super) fn dry_run(argv: &[String]) -> Result<String, String> {
        let endpoint = &argv[2];
        let mut operation = PolicyOperation::default();
        let mut protocol = PolicyProtocol::default();
        for pair in argv[3..].chunks(2) {
            if pair.len() != 2 {
                return Err(Self::usage());
            }
            match pair[0].as_str() {
                "--operation" => operation = PolicyOperation::parse(&pair[1])?,
                "--protocol" => protocol = PolicyProtocol::parse(&pair[1])?,
                _ => return Err(Self::usage()),
            }
        }
        let result =
            crate::actrail::plugin::network_control_host::network_policy_rules_match_dry_run(
                &NetworkPolicyMatchDryRunRequest {
                    operation: operation.host_value(),
                    protocol: protocol.host_value(),
                    endpoint: endpoint.to_string(),
                },
            )?;
        Ok(format!(
            "matched={} decision={} rule_id={} owner={} endpoint={} rule_revision={} source_revision={}\n",
            result.matched,
            Self::host_decision_name(result.decision),
            result.rule_id.unwrap_or_else(|| "none".to_string()),
            result
                .owner_instance_id
                .unwrap_or_else(|| "none".to_string()),
            result.resolved_endpoint,
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

    pub(super) fn usage() -> String {
        "usage: help | rule list | rule dry-run <ip:port> [--operation OP] [--protocol PROTOCOL] | rule upsert <allow|deny|gray> <ip-or-cidr:port-or-range-or-*> [--operation OP] [--protocol PROTOCOL] [--rule-id ID] [--gray-target INSTANCE --timeout-ms N --concurrency N --fallback allow|deny] | rule delete <rule-id>".to_string()
    }

    pub(super) fn help() -> String {
        "supported commands:\n  help\n  rule list\n  rule dry-run <ip:port> [--operation OP] [--protocol PROTOCOL]\n  rule upsert <allow|deny|gray> <ip-or-cidr:port-or-range-or-*> [--operation OP] [--protocol PROTOCOL] [--rule-id ID] [--gray-target INSTANCE --timeout-ms N --concurrency N --fallback allow|deny]\n  rule delete <rule-id>\n".to_string()
    }
}
