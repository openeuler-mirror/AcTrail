//! WIT value conversion and bounded-size accounting for managed network policy.

use plugin_system::{
    ControlVerdict, NetworkActionContext, NetworkPolicyApplyRequest, NetworkPolicyApplyResult,
    NetworkPolicyApplyStatus, NetworkPolicyDecision, NetworkPolicyListFilter,
    NetworkPolicyListResult, NetworkPolicyMatchDryRunRequest, NetworkPolicyMatchDryRunResult,
    NetworkPolicyPatchItem, NetworkPolicyPatchOp, NetworkPolicyRuleDraft, NetworkPolicyRuleView,
};
use wasmtime::component::Val;

use super::super::component_abi;
use super::super::value::{
    component_field_enum, component_field_list, component_field_option,
    component_field_option_string, component_field_string, component_field_u64,
    component_option_string, component_option_u32, component_option_u64,
};

pub(super) struct NetworkComponentCodec;

impl NetworkComponentCodec {
    pub(super) fn parse_list_filter(
        fields: &[(String, Val)],
    ) -> Result<NetworkPolicyListFilter, String> {
        let decision =
            match component_field_option(fields, component_abi::network_policy::DECISION)? {
                Some(Val::Enum(value)) => Some(NetworkPolicyDecision::from_wire(value)?),
                Some(other) => {
                    return Err(format!(
                        "network policy decision filter must be an enum, got {other:?}"
                    ));
                }
                None => None,
            };
        Ok(NetworkPolicyListFilter {
            decision,
            remote: component_field_option_string(fields, component_abi::network_policy::REMOTE)?
                .map(str::to_string),
        })
    }

    pub(super) fn parse_match_request(
        fields: &[(String, Val)],
    ) -> Result<NetworkPolicyMatchDryRunRequest, String> {
        Ok(NetworkPolicyMatchDryRunRequest {
            remote: component_field_string(fields, component_abi::network_policy::REMOTE)?
                .to_string(),
        })
    }

    pub(super) fn parse_apply_request(
        fields: &[(String, Val)],
    ) -> Result<NetworkPolicyApplyRequest, String> {
        Ok(NetworkPolicyApplyRequest {
            base_revision: component_field_u64(fields, "base-revision")?,
            mutation_id: component_field_string(fields, "mutation-id")?.to_string(),
            reason: component_field_option_string(fields, "reason")?.map(str::to_string),
            items: component_field_list(fields, "items")?
                .iter()
                .map(Self::parse_patch_item)
                .collect::<Result<Vec<_>, _>>()?,
        })
    }

    fn parse_patch_item(value: &Val) -> Result<NetworkPolicyPatchItem, String> {
        let Val::Record(fields) = value else {
            return Err("network policy patch item must be a record".to_string());
        };
        let op = match component_field_enum(fields, "op")? {
            "upsert" => NetworkPolicyPatchOp::Upsert,
            "delete" => NetworkPolicyPatchOp::Delete,
            other => return Err(format!("unsupported network policy patch op {other}")),
        };
        let rule_id = component_field_option_string(fields, "rule-id")?.map(str::to_string);
        let rule = match component_field_option(fields, "rule")? {
            Some(Val::Record(rule_fields)) => Some(Self::parse_rule_draft(rule_fields)?),
            Some(other) => {
                return Err(format!(
                    "network policy patch rule must be a record, got {other:?}"
                ));
            }
            None => None,
        };
        Ok(NetworkPolicyPatchItem { op, rule_id, rule })
    }

    fn parse_rule_draft(fields: &[(String, Val)]) -> Result<NetworkPolicyRuleDraft, String> {
        Ok(NetworkPolicyRuleDraft {
            rule_id: component_field_option_string(fields, "rule-id")?.map(str::to_string),
            decision: NetworkPolicyDecision::from_wire(component_field_enum(fields, "decision")?)?,
            remote: component_field_string(fields, "remote")?.to_string(),
            gray_target: component_field_option_string(fields, "gray-target")?.map(str::to_string),
            timeout_ms: Self::option_u64(fields, "timeout-ms")?,
            concurrency_limit: Self::option_u32(fields, "concurrency-limit")?,
            fallback: Self::option_verdict(fields, "fallback")?,
        })
    }

    pub(super) fn encode_context(context: &NetworkActionContext) -> Val {
        Val::Record(vec![
            ("syscall".to_string(), Val::String(context.syscall.clone())),
            ("fd".to_string(), Val::U64(context.fd)),
            (
                "address-family".to_string(),
                Val::String(context.address_family.clone()),
            ),
            (
                "remote-address".to_string(),
                Val::String(context.remote_address.clone()),
            ),
            ("remote-port".to_string(), Val::U16(context.remote_port)),
            ("ipv6-scope-id".to_string(), Val::U32(context.ipv6_scope_id)),
        ])
    }

    pub(super) fn encode_list_result(result: NetworkPolicyListResult) -> Val {
        Val::Record(vec![
            (
                "rules".to_string(),
                Val::List(
                    result
                        .rules
                        .into_iter()
                        .map(Self::encode_rule_view)
                        .collect(),
                ),
            ),
            (
                "next-cursor".to_string(),
                component_option_string(result.next_cursor),
            ),
            (
                "source-revision".to_string(),
                Val::U64(result.source_revision),
            ),
        ])
    }

    fn encode_rule_view(rule: NetworkPolicyRuleView) -> Val {
        Val::Record(vec![
            ("rule-id".to_string(), Val::String(rule.rule_id)),
            (
                "owner-instance-id".to_string(),
                Val::String(rule.owner_instance_id),
            ),
            (
                "decision".to_string(),
                Val::Enum(rule.decision.as_str().to_string()),
            ),
            ("remote".to_string(), Val::String(rule.remote)),
            (
                "gray-target".to_string(),
                component_option_string(rule.gray_target),
            ),
            (
                "timeout-ms".to_string(),
                component_option_u64(rule.timeout_ms),
            ),
            (
                "concurrency-limit".to_string(),
                component_option_u32(rule.concurrency_limit),
            ),
            (
                "fallback".to_string(),
                Self::encode_option_verdict(rule.fallback),
            ),
            ("rule-revision".to_string(), Val::U64(rule.rule_revision)),
            (
                "updated-sequence".to_string(),
                Val::U64(rule.updated_sequence),
            ),
        ])
    }

    pub(super) fn encode_match_result(result: NetworkPolicyMatchDryRunResult) -> Val {
        Val::Record(vec![
            ("matched".to_string(), Val::Bool(result.matched)),
            (
                "decision".to_string(),
                Val::Enum(result.decision.as_str().to_string()),
            ),
            (
                "rule-id".to_string(),
                component_option_string(result.rule_id),
            ),
            (
                "owner-instance-id".to_string(),
                component_option_string(result.owner_instance_id),
            ),
            (
                "resolved-remote".to_string(),
                Val::String(result.resolved_remote),
            ),
            (
                "rule-revision".to_string(),
                component_option_u64(result.rule_revision),
            ),
            (
                "source-revision".to_string(),
                Val::U64(result.source_revision),
            ),
        ])
    }

    pub(super) fn encode_apply_result(result: NetworkPolicyApplyResult) -> Val {
        Val::Record(vec![
            (
                "status".to_string(),
                Val::Enum(
                    match result.status {
                        NetworkPolicyApplyStatus::Accepted => "accepted",
                        NetworkPolicyApplyStatus::Rejected => "rejected",
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

    fn option_u64(fields: &[(String, Val)], name: &str) -> Result<Option<u64>, String> {
        match component_field_option(fields, name)? {
            Some(Val::U64(value)) => Ok(Some(*value)),
            Some(other) => Err(format!("field {name} must contain u64, got {other:?}")),
            None => Ok(None),
        }
    }

    fn option_u32(fields: &[(String, Val)], name: &str) -> Result<Option<u32>, String> {
        match component_field_option(fields, name)? {
            Some(Val::U32(value)) => Ok(Some(*value)),
            Some(other) => Err(format!("field {name} must contain u32, got {other:?}")),
            None => Ok(None),
        }
    }

    fn option_verdict(
        fields: &[(String, Val)],
        name: &str,
    ) -> Result<Option<ControlVerdict>, String> {
        match component_field_option(fields, name)? {
            Some(Val::Enum(value)) if value == "allow" => Ok(Some(ControlVerdict::Allow)),
            Some(Val::Enum(value)) if value == "deny" => Ok(Some(ControlVerdict::Deny)),
            Some(other) => Err(format!(
                "field {name} must contain allow or deny, got {other:?}"
            )),
            None => Ok(None),
        }
    }

    fn encode_option_verdict(verdict: Option<ControlVerdict>) -> Val {
        Val::Option(verdict.map(|value| Box::new(Val::Enum(value.as_str().to_string()))))
    }

    pub(super) fn context_size(context: &NetworkActionContext) -> usize {
        context
            .syscall
            .len()
            .saturating_add(context.address_family.len())
            .saturating_add(context.remote_address.len())
    }

    pub(super) fn apply_request_size(request: &NetworkPolicyApplyRequest) -> usize {
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
                            .saturating_add(rule.remote.len())
                            .saturating_add(rule.gray_target.as_ref().map_or(0, String::len))
                    }))
            }))
    }

    pub(super) fn apply_result_size(result: &NetworkPolicyApplyResult) -> usize {
        result.errors.iter().fold(0_usize, |size, error| {
            size.saturating_add(error.code.len())
                .saturating_add(error.message.len())
        })
    }

    pub(super) fn list_result_size(result: &NetworkPolicyListResult) -> usize {
        result
            .next_cursor
            .as_ref()
            .map_or(0, String::len)
            .saturating_add(result.rules.iter().fold(0_usize, |size, rule| {
                size.saturating_add(rule.rule_id.len())
                    .saturating_add(rule.owner_instance_id.len())
                    .saturating_add(rule.remote.len())
                    .saturating_add(rule.gray_target.as_ref().map_or(0, String::len))
            }))
    }

    pub(super) fn match_result_size(result: &NetworkPolicyMatchDryRunResult) -> usize {
        result
            .resolved_remote
            .len()
            .saturating_add(result.rule_id.as_ref().map_or(0, String::len))
            .saturating_add(result.owner_instance_id.as_ref().map_or(0, String::len))
    }
}
