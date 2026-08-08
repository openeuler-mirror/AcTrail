use plugin_system::{CommandPolicyDecision, CommandPolicyRuleDraft};

use super::{STATIC_POLICY_OWNER, StoredCommandRule};
use crate::services::command_control::decision::CommandRuleDraftValidator;

pub(super) struct StaticRuleParser;

impl StaticRuleParser {
    pub(super) fn parse(line: &str, sequence: u64) -> Result<StoredCommandRule, String> {
        let (scope, priority) = line.rsplit_once(" priority ").ok_or_else(Self::usage)?;
        let (head, args) =
            match scope.split_once(" args-json ") {
                Some((head, raw)) => (
                    head,
                    Some(serde_json::from_str::<Vec<String>>(raw).map_err(|error| {
                        format!("args-json must be a JSON string array: {error}")
                    })?),
                ),
                None => (scope, None),
            };
        let fields = head.split_whitespace().collect::<Vec<_>>();
        let (rule_id, decision, executable, gray_target) = match fields.as_slice() {
            [rule_id, decision @ ("allow" | "deny"), "exec", executable] => {
                (*rule_id, *decision, *executable, None)
            }
            [rule_id, "gray", "sync-plugin", target, "exec", executable] => {
                (*rule_id, "gray", *executable, Some((*target).to_string()))
            }
            _ => return Err(Self::usage()),
        };
        CommandRuleDraftValidator::validate_id(rule_id)?;
        let decision = CommandPolicyDecision::from_wire(decision)?;
        let draft = CommandPolicyRuleDraft {
            rule_id: Some(rule_id.to_string()),
            decision,
            executable: executable.to_string(),
            args,
            gray_target,
            priority: priority
                .parse::<i32>()
                .map_err(|error| format!("priority must be an i32: {error}"))?,
        };
        StoredCommandRule::from_draft(STATIC_POLICY_OWNER, Some(rule_id), &draft, 0, sequence, 0)
    }

    fn usage() -> String {
        "expected: <id> <allow|deny> exec <absolute-path> [args-json <json-string-array>] priority <i32> or <id> gray sync-plugin <instance-id> exec <absolute-path> [args-json <json-string-array>] priority <i32>".to_string()
    }
}
