//! WIT command-execution context and command-policy record codec.

use plugin_system::{
    CommandExecutionContext, CommandPolicyApplyRequest, CommandPolicyApplyResult,
    CommandPolicyApplyStatus, CommandPolicyDecision, CommandPolicyListFilter,
    CommandPolicyListResult, CommandPolicyMatchDryRunRequest, CommandPolicyMatchDryRunResult,
    CommandPolicyPatchItem, CommandPolicyPatchOp, CommandPolicyRuleDraft, CommandPolicyRuleView,
};
use wasmtime::component::Val;

use super::component_abi;
use super::value::{
    component_field_enum, component_field_list, component_field_option,
    component_field_option_string, component_field_s32, component_field_string,
    component_field_u64, component_option_string, component_option_u64,
};

pub(super) struct CommandContextWireCodec;

impl CommandContextWireCodec {
    pub(super) fn encode(context: &CommandExecutionContext) -> Val {
        Val::Record(vec![
            ("syscall".to_string(), Val::String(context.syscall.clone())),
            (
                "requested-path".to_string(),
                Val::String(context.requested_path.clone()),
            ),
            (
                "resolved-path".to_string(),
                Val::String(context.resolved_path.clone()),
            ),
            (
                "argv".to_string(),
                Val::List(context.argv.iter().cloned().map(Val::String).collect()),
            ),
            (
                "execveat-dirfd".to_string(),
                Val::Option(
                    context
                        .execveat_dirfd
                        .map(|value| Box::new(Val::S32(value))),
                ),
            ),
            (
                "execveat-flags".to_string(),
                component_option_u64(context.execveat_flags),
            ),
        ])
    }

    pub(super) fn size(context: &CommandExecutionContext) -> usize {
        context
            .syscall
            .len()
            .saturating_add(context.requested_path.len())
            .saturating_add(context.resolved_path.len())
            .saturating_add(
                context
                    .argv
                    .iter()
                    .map(String::len)
                    .fold(0_usize, usize::saturating_add),
            )
    }
}

pub(super) struct CommandPolicyWireCodec;

impl CommandPolicyWireCodec {
    pub(super) fn parse_list_filter(
        fields: &[(String, Val)],
    ) -> Result<CommandPolicyListFilter, String> {
        let decision =
            match component_field_option(fields, component_abi::command_policy::DECISION)? {
                Some(Val::Enum(value)) => Some(CommandPolicyDecision::from_wire(value)?),
                Some(other) => {
                    return Err(format!(
                        "command policy decision filter must be an enum, got {other:?}"
                    ));
                }
                None => None,
            };
        Ok(CommandPolicyListFilter {
            decision,
            executable_prefix: component_field_option_string(
                fields,
                component_abi::command_policy::EXECUTABLE_PREFIX,
            )?
            .map(str::to_string),
        })
    }

    pub(super) fn parse_match_request(
        fields: &[(String, Val)],
    ) -> Result<CommandPolicyMatchDryRunRequest, String> {
        Ok(CommandPolicyMatchDryRunRequest {
            executable: component_field_string(fields, component_abi::command_policy::EXECUTABLE)?
                .to_string(),
            args: Self::parse_string_list(
                component_field_list(fields, component_abi::command_policy::ARGS)?,
                "command policy dry-run args",
            )?,
        })
    }

    pub(super) fn parse_apply_request(
        fields: &[(String, Val)],
    ) -> Result<CommandPolicyApplyRequest, String> {
        Ok(CommandPolicyApplyRequest {
            base_revision: component_field_u64(fields, "base-revision")?,
            mutation_id: component_field_string(fields, "mutation-id")?.to_string(),
            reason: component_field_option_string(fields, "reason")?.map(str::to_string),
            items: component_field_list(fields, "items")?
                .iter()
                .map(Self::parse_patch_item)
                .collect::<Result<Vec<_>, _>>()?,
        })
    }

    pub(super) fn parse_patch_item(value: &Val) -> Result<CommandPolicyPatchItem, String> {
        let Val::Record(fields) = value else {
            return Err("command policy patch item must be a record".to_string());
        };
        let op = match component_field_enum(fields, "op")? {
            "upsert" => CommandPolicyPatchOp::Upsert,
            "delete" => CommandPolicyPatchOp::Delete,
            other => return Err(format!("unsupported command policy patch op {other}")),
        };
        let rule_id = component_field_option_string(fields, "rule-id")?.map(str::to_string);
        let rule = match component_field_option(fields, "rule")? {
            Some(Val::Record(rule_fields)) => Some(Self::parse_rule_draft(rule_fields)?),
            Some(other) => {
                return Err(format!(
                    "command policy patch rule must be a record, got {other:?}"
                ));
            }
            None => None,
        };
        Ok(CommandPolicyPatchItem { op, rule_id, rule })
    }

    pub(super) fn parse_rule_draft(
        fields: &[(String, Val)],
    ) -> Result<CommandPolicyRuleDraft, String> {
        Ok(CommandPolicyRuleDraft {
            rule_id: component_field_option_string(fields, "rule-id")?.map(str::to_string),
            decision: CommandPolicyDecision::from_wire(component_field_enum(fields, "decision")?)?,
            executable: component_field_string(fields, "executable")?.to_string(),
            args: Self::parse_optional_args(fields)?,
            gray_target: component_field_option_string(fields, "gray-target")?.map(str::to_string),
            priority: component_field_s32(fields, "priority")?,
        })
    }

    pub(super) fn encode_list_result(result: CommandPolicyListResult) -> Val {
        Val::Record(vec![
            (
                component_abi::command_policy::RULES.to_string(),
                Val::List(
                    result
                        .rules
                        .into_iter()
                        .map(Self::encode_rule_view)
                        .collect(),
                ),
            ),
            (
                component_abi::command_policy::NEXT_CURSOR.to_string(),
                component_option_string(result.next_cursor),
            ),
            (
                component_abi::command_policy::SOURCE_REVISION.to_string(),
                Val::U64(result.source_revision),
            ),
        ])
    }

    pub(super) fn encode_rule_view(rule: CommandPolicyRuleView) -> Val {
        Val::Record(vec![
            (
                component_abi::command_policy::RULE_ID.to_string(),
                Val::String(rule.rule_id),
            ),
            (
                component_abi::command_policy::OWNER_INSTANCE_ID.to_string(),
                Val::String(rule.owner_instance_id),
            ),
            (
                component_abi::command_policy::DECISION.to_string(),
                Val::Enum(rule.decision.as_str().to_string()),
            ),
            (
                component_abi::command_policy::EXECUTABLE.to_string(),
                Val::String(rule.executable),
            ),
            (
                component_abi::command_policy::ARGS.to_string(),
                Val::Option(
                    rule.args.map(|args| {
                        Box::new(Val::List(args.into_iter().map(Val::String).collect()))
                    }),
                ),
            ),
            (
                component_abi::command_policy::GRAY_TARGET.to_string(),
                component_option_string(rule.gray_target),
            ),
            (
                component_abi::command_policy::PRIORITY.to_string(),
                Val::S32(rule.priority),
            ),
            (
                component_abi::command_policy::RULE_REVISION.to_string(),
                Val::U64(rule.rule_revision),
            ),
            (
                component_abi::command_policy::UPDATED_SEQUENCE.to_string(),
                Val::U64(rule.updated_sequence),
            ),
        ])
    }

    pub(super) fn encode_match_result(result: CommandPolicyMatchDryRunResult) -> Val {
        Val::Record(vec![
            (
                component_abi::command_policy::MATCHED.to_string(),
                Val::Bool(result.matched),
            ),
            (
                "decision".to_string(),
                Val::Enum(result.decision.as_str().to_string()),
            ),
            (
                component_abi::command_policy::RULE_ID.to_string(),
                component_option_string(result.rule_id),
            ),
            (
                component_abi::command_policy::OWNER_INSTANCE_ID.to_string(),
                component_option_string(result.owner_instance_id),
            ),
            (
                component_abi::command_policy::RESOLVED_EXECUTABLE.to_string(),
                Val::String(result.resolved_executable),
            ),
            (
                component_abi::command_policy::RULE_REVISION.to_string(),
                component_option_u64(result.rule_revision),
            ),
            (
                "source-revision".to_string(),
                Val::U64(result.source_revision),
            ),
        ])
    }

    pub(super) fn encode_apply_result(result: CommandPolicyApplyResult) -> Val {
        Val::Record(vec![
            (
                "status".to_string(),
                Val::Enum(
                    match result.status {
                        CommandPolicyApplyStatus::Accepted => "accepted",
                        CommandPolicyApplyStatus::Rejected => "rejected",
                    }
                    .to_string(),
                ),
            ),
            ("new-revision".to_string(), Val::U64(result.new_revision)),
            ("applied-count".to_string(), Val::U32(result.applied_count)),
            (
                "rejected-count".to_string(),
                Val::U32(result.rejected_count),
            ),
            (
                "errors".to_string(),
                Val::List(
                    result
                        .errors
                        .into_iter()
                        .map(|error| {
                            Val::Record(vec![
                                ("item-index".to_string(), Val::U32(error.item_index)),
                                ("code".to_string(), Val::String(error.code)),
                                ("message".to_string(), Val::String(error.message)),
                            ])
                        })
                        .collect(),
                ),
            ),
        ])
    }

    pub(super) fn apply_request_size(request: &CommandPolicyApplyRequest) -> usize {
        request
            .mutation_id
            .len()
            .saturating_add(request.reason.as_ref().map_or(0, String::len))
            .saturating_add(request.items.iter().fold(0_usize, |size, item| {
                size.saturating_add(item.rule_id.as_ref().map_or(0, String::len))
                    .saturating_add(item.rule.as_ref().map_or(0, |rule| {
                        rule.rule_id
                            .as_ref()
                            .map_or(0, String::len)
                            .saturating_add(rule.executable.len())
                            .saturating_add(Self::optional_args_size(rule.args.as_ref()))
                            .saturating_add(rule.gray_target.as_ref().map_or(0, String::len))
                    }))
            }))
    }

    pub(super) fn apply_result_size(result: &CommandPolicyApplyResult) -> usize {
        result.errors.iter().fold(0_usize, |size, error| {
            size.saturating_add(error.code.len())
                .saturating_add(error.message.len())
        })
    }

    pub(super) fn list_result_size(result: &CommandPolicyListResult) -> usize {
        result
            .next_cursor
            .as_ref()
            .map_or(0, String::len)
            .saturating_add(result.rules.iter().fold(0_usize, |size, rule| {
                size.saturating_add(rule.rule_id.len())
                    .saturating_add(rule.owner_instance_id.len())
                    .saturating_add(rule.executable.len())
                    .saturating_add(Self::optional_args_size(rule.args.as_ref()))
                    .saturating_add(rule.gray_target.as_ref().map_or(0, String::len))
            }))
    }

    pub(super) fn match_result_size(result: &CommandPolicyMatchDryRunResult) -> usize {
        result
            .resolved_executable
            .len()
            .saturating_add(result.rule_id.as_ref().map_or(0, String::len))
            .saturating_add(result.owner_instance_id.as_ref().map_or(0, String::len))
    }

    pub(super) fn match_request_size(request: &CommandPolicyMatchDryRunRequest) -> usize {
        request
            .args
            .iter()
            .fold(request.executable.len(), |size, arg| {
                size.saturating_add(arg.len())
            })
    }

    fn parse_optional_args(fields: &[(String, Val)]) -> Result<Option<Vec<String>>, String> {
        match component_field_option(fields, component_abi::command_policy::ARGS)? {
            Some(Val::List(values)) => {
                Self::parse_string_list(values, "command policy rule args").map(Some)
            }
            Some(other) => Err(format!(
                "command policy rule args must be a string list, got {other:?}"
            )),
            None => Ok(None),
        }
    }

    fn parse_string_list(values: &[Val], label: &str) -> Result<Vec<String>, String> {
        values
            .iter()
            .map(|value| match value {
                Val::String(value) => Ok(value.clone()),
                other => Err(format!("{label} must contain strings, got {other:?}")),
            })
            .collect()
    }

    fn optional_args_size(args: Option<&Vec<String>>) -> usize {
        args.map_or(0, |values| {
            values
                .iter()
                .fold(0_usize, |size, value| size.saturating_add(value.len()))
        })
    }
}
