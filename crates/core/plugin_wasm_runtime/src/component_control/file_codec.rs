//! WIT file-policy request parsing and response record codec.

use plugin_system::{
    FilePolicyApplyMode, FilePolicyApplyPrecondition, FilePolicyApplyRequest,
    FilePolicyApplyResult, FilePolicyApplyStatus, FilePolicyDecision, FilePolicyListFilter,
    FilePolicyListResult, FilePolicyMatchDryRunRequest, FilePolicyMatchDryRunResult,
    FilePolicyOperation, FilePolicyPatchItem, FilePolicyPatchOp, FilePolicyRuleDraft,
    FilePolicyRuleView,
};
use wasmtime::component::Val;

use super::component_abi;
use super::value::{
    component_field_enum, component_field_list, component_field_option,
    component_field_option_string, component_field_option_u64, component_field_s32,
    component_field_string, component_field_u64, component_option_string, component_option_u64,
};

pub(super) fn parse_component_file_policy_list_filter(
    fields: &[(String, Val)],
) -> Result<FilePolicyListFilter, String> {
    let decision =
        match component_field_option(fields, component_abi::file_policy_list_filter::DECISION)? {
            Some(Val::Enum(value)) => Some(parse_component_file_policy_decision(value)?),
            Some(other) => {
                return Err(format!(
                    "field {} option must contain enum, got {other:?}",
                    component_abi::file_policy_list_filter::DECISION
                ));
            }
            None => None,
        };
    let path_prefix =
        component_field_option_string(fields, component_abi::file_policy_list_filter::PATH_PREFIX)?
            .map(str::to_string);
    let operation =
        match component_field_option(fields, component_abi::file_policy_list_filter::OPERATION)? {
            Some(Val::Enum(value)) => Some(parse_component_file_policy_operation(value)?),
            Some(other) => {
                return Err(format!(
                    "field {} option must contain enum, got {other:?}",
                    component_abi::file_policy_list_filter::OPERATION
                ));
            }
            None => None,
        };
    Ok(FilePolicyListFilter {
        decision,
        path_prefix,
        operation,
    })
}

pub(super) fn parse_component_file_policy_match_dry_run_request(
    fields: &[(String, Val)],
) -> Result<FilePolicyMatchDryRunRequest, String> {
    Ok(FilePolicyMatchDryRunRequest {
        path: component_field_string(fields, component_abi::file_policy_match_dry_run::PATH)?
            .to_string(),
        operation: parse_component_file_policy_operation(component_field_enum(
            fields,
            component_abi::file_policy_match_dry_run::OPERATION,
        )?)?,
    })
}

pub(super) fn component_file_policy_list_result(result: FilePolicyListResult) -> Val {
    Val::Record(vec![
        (
            component_abi::file_policy_list_result::RULES.to_string(),
            Val::List(
                result
                    .rules
                    .into_iter()
                    .map(component_file_policy_rule_view)
                    .collect(),
            ),
        ),
        (
            component_abi::file_policy_list_result::NEXT_CURSOR.to_string(),
            component_option_string(result.next_cursor),
        ),
        (
            component_abi::file_policy_list_result::SOURCE_REVISION.to_string(),
            Val::U64(result.source_revision),
        ),
    ])
}

fn component_file_policy_rule_view(rule: FilePolicyRuleView) -> Val {
    Val::Record(vec![
        (
            component_abi::file_policy_rule_view::RULE_ID.to_string(),
            Val::String(rule.rule_id),
        ),
        (
            component_abi::file_policy_rule_view::OWNER_INSTANCE_ID.to_string(),
            Val::String(rule.owner_instance_id),
        ),
        (
            component_abi::file_policy_rule_view::DECISION.to_string(),
            component_file_policy_decision(rule.decision),
        ),
        (
            component_abi::file_policy_rule_view::OPERATION.to_string(),
            component_file_policy_operation(rule.operation),
        ),
        (
            component_abi::file_policy_rule_view::PATH.to_string(),
            Val::String(rule.path),
        ),
        (
            component_abi::file_policy_rule_view::GRAY_TARGET.to_string(),
            component_option_u64(rule.gray_target),
        ),
        (
            component_abi::file_policy_rule_view::PRIORITY.to_string(),
            Val::S32(rule.priority),
        ),
        (
            component_abi::file_policy_rule_view::ENABLED.to_string(),
            Val::Bool(rule.enabled),
        ),
        (
            component_abi::file_policy_rule_view::UPDATED_SEQUENCE.to_string(),
            Val::U64(rule.updated_sequence),
        ),
    ])
}

pub(super) fn component_file_policy_match_dry_run_result(
    result: FilePolicyMatchDryRunResult,
) -> Val {
    Val::Record(vec![
        (
            component_abi::file_policy_match_dry_run::MATCHED.to_string(),
            Val::Bool(result.matched),
        ),
        (
            component_abi::file_policy_match_dry_run::DECISION.to_string(),
            component_file_policy_decision(result.decision),
        ),
        (
            component_abi::file_policy_match_dry_run::RULE_ID.to_string(),
            component_option_string(result.rule_id),
        ),
        (
            component_abi::file_policy_match_dry_run::OPERATION.to_string(),
            component_file_policy_operation(result.operation),
        ),
        (
            component_abi::file_policy_match_dry_run::CANONICAL_PATH.to_string(),
            Val::String(result.canonical_path),
        ),
        (
            component_abi::file_policy_match_dry_run::SOURCE_REVISION.to_string(),
            Val::U64(result.source_revision),
        ),
    ])
}

fn component_file_policy_decision(decision: FilePolicyDecision) -> Val {
    Val::Enum(decision.as_str().to_string())
}

fn component_file_policy_operation(operation: FilePolicyOperation) -> Val {
    Val::Enum(operation.as_str().to_string())
}

pub(super) fn parse_component_file_policy_apply_request(
    fields: &[(String, Val)],
) -> Result<FilePolicyApplyRequest, String> {
    let base_revision = component_field_u64(fields, "base-revision")?;
    let mutation_id = component_field_string(fields, "mutation-id")?.to_string();
    let reason = component_field_option_string(fields, "reason")?.map(str::to_string);
    let correlation_id =
        component_field_option_string(fields, "correlation-id")?.map(str::to_string);
    let apply_mode = match component_field_enum(fields, "apply-mode")? {
        "partial" => FilePolicyApplyMode::Partial,
        "aon" => FilePolicyApplyMode::Aon,
        other => return Err(format!("unsupported apply-mode {other}")),
    };
    let items = component_field_list(fields, "items")?
        .iter()
        .map(parse_component_file_policy_patch_item)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(FilePolicyApplyRequest {
        items,
        precondition: FilePolicyApplyPrecondition {
            base_revision,
            mutation_id,
            reason,
            correlation_id,
            apply_mode,
        },
    })
}

fn parse_component_file_policy_patch_item(value: &Val) -> Result<FilePolicyPatchItem, String> {
    let Val::Record(fields) = value else {
        return Err("file-policy patch item must be a record".to_string());
    };
    let op = match component_field_enum(fields, "op")? {
        "upsert" => FilePolicyPatchOp::Upsert,
        "delete" => FilePolicyPatchOp::Delete,
        "enable" => FilePolicyPatchOp::Enable,
        "disable" => FilePolicyPatchOp::Disable,
        other => return Err(format!("unsupported patch op {other}")),
    };
    let rule_id = component_field_option_string(fields, "rule-id")?.map(str::to_string);
    let rule = match component_field_option(fields, "rule")? {
        Some(Val::Record(rule_fields)) => {
            Some(parse_component_file_policy_rule_draft(rule_fields)?)
        }
        Some(_) => return Err("file-policy patch rule must be a record".to_string()),
        None => None,
    };
    Ok(FilePolicyPatchItem { op, rule_id, rule })
}

fn parse_component_file_policy_rule_draft(
    fields: &[(String, Val)],
) -> Result<FilePolicyRuleDraft, String> {
    let rule_id = component_field_option_string(fields, "rule-id")?.map(str::to_string);
    let decision = parse_component_file_policy_decision(component_field_enum(fields, "decision")?)?;
    let operation =
        parse_component_file_policy_operation(component_field_enum(fields, "operation")?)?;
    Ok(FilePolicyRuleDraft {
        rule_id,
        decision,
        operation,
        path: component_field_string(fields, "path")?.to_string(),
        gray_target: component_field_option_u64(fields, "gray-target")?,
        priority: component_field_s32(fields, "priority")?,
    })
}

pub(super) fn component_file_policy_apply_result(result: FilePolicyApplyResult) -> Val {
    Val::Record(vec![
        (
            "status".to_string(),
            Val::Enum(
                match result.status {
                    FilePolicyApplyStatus::Accepted => "accepted",
                    FilePolicyApplyStatus::Rejected => "rejected",
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

fn parse_component_file_policy_decision(value: &str) -> Result<FilePolicyDecision, String> {
    FilePolicyDecision::from_wire(value)
}

fn parse_component_file_policy_operation(value: &str) -> Result<FilePolicyOperation, String> {
    FilePolicyOperation::from_wire(value)
}
