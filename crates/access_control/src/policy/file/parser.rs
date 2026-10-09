use super::scope::PathScope;
use super::types::{EnforcementRule, RuleDecision, STATIC_POLICY_OWNER};
use config_core::daemon::EnforcementDecision;
use plugin_system::FilePolicyOperation;

pub(super) struct FileRuleParser;
impl FileRuleParser {
    pub(super) fn parse_rule(raw: &str) -> Result<EnforcementRule, String> {
        let fields = raw.split_whitespace().collect::<Vec<_>>();
        match fields.as_slice() {
            [rule_id, decision, operation, path] => Self::parse_local_rule(rule_id, decision, operation, path),
            [
                rule_id,
                "gray",
                "sync-plugin",
                instance_id,
                "timeout-ms",
                timeout_ms,
                "concurrency",
                concurrency_limit,
                "fallback",
                fallback,
                operation,
                path,
            ] => Self::parse_sync_plugin_rule(
                rule_id,
                instance_id,
                timeout_ms,
                concurrency_limit,
                fallback,
                operation,
                path,
            ),
            _ => Err(
                "expected: <rule_id> <allow|deny> <any|open|mkdir|rmdir> <absolute-path> or <rule_id> gray sync-plugin <instance> timeout-ms <positive-ms> concurrency <positive-limit> fallback <allow|deny> <any|open|mkdir|rmdir> <absolute-path>"
                    .to_string(),
            ),
        }
    }

    fn parse_local_rule(
        rule_id: &str,
        decision: &str,
        operation: &str,
        path: &str,
    ) -> Result<EnforcementRule, String> {
        let operation = FilePolicyOperation::from_wire(operation)?;
        Ok(EnforcementRule {
            owner_instance_id: STATIC_POLICY_OWNER.to_string(),
            rule_id: rule_id.to_string(),
            decision: RuleDecision::Local(decision.parse::<EnforcementDecision>()?),
            operation: operation.as_str().to_string(),
            path: PathScope::absolute_path(path)?,
        })
    }

    fn parse_sync_plugin_rule(
        rule_id: &str,
        instance_id: &str,
        timeout_ms: &str,
        concurrency_limit: &str,
        fallback: &str,
        operation: &str,
        path: &str,
    ) -> Result<EnforcementRule, String> {
        let operation = FilePolicyOperation::from_wire(operation)?;
        if instance_id.trim().is_empty() {
            return Err("sync-plugin instance id must not be empty".to_string());
        }
        Ok(EnforcementRule {
            owner_instance_id: STATIC_POLICY_OWNER.to_string(),
            rule_id: rule_id.to_string(),
            decision: RuleDecision::SyncPlugin {
                instance_id: instance_id.to_string(),
                timeout_ms: Self::parse_positive_u64("timeout-ms", timeout_ms)?,
                concurrency_limit: Self::parse_positive_u32("concurrency", concurrency_limit)?,
                fallback: fallback.parse::<EnforcementDecision>()?,
            },
            operation: operation.as_str().to_string(),
            path: PathScope::absolute_path(path)?,
        })
    }

    fn parse_positive_u64(name: &str, raw: &str) -> Result<u64, String> {
        let value = raw
            .parse::<u64>()
            .map_err(|error| format!("{name} must be a positive integer: {error}"))?;
        if value == 0 {
            return Err(format!("{name} must be greater than zero"));
        }
        Ok(value)
    }

    fn parse_positive_u32(name: &str, raw: &str) -> Result<u32, String> {
        let value = raw
            .parse::<u32>()
            .map_err(|error| format!("{name} must be a positive integer: {error}"))?;
        if value == 0 {
            return Err(format!("{name} must be greater than zero"));
        }
        Ok(value)
    }
}
