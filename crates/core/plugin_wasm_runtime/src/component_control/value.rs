//! Shared WIT component values, field decoding, and control response parsing.

use plugin_system::{
    ControlDecisionRequest, ControlDecisionResponse, ControlVerdict, DecisionScope,
    PluginCommandRequest, PluginCommandResponse, PluginRuntimeError,
};
use wasmtime::component::Val;

use super::component_abi;

pub(super) fn set_component_string_error(results: &mut [Val], message: &str) {
    let Some(result) = results.first_mut() else {
        return;
    };
    *result = Val::Result(Err(Some(Box::new(Val::String(message.to_string())))));
}

pub(super) fn set_component_val_ok(results: &mut [Val], value: Val) {
    let Some(result) = results.first_mut() else {
        return;
    };
    *result = Val::Result(Ok(Some(Box::new(value))));
}

pub(super) fn decision_summary_val(context: &crate::engine::ControlContextSnapshot) -> Val {
    Val::Record(vec![
        (
            component_abi::decision_summary::SUBJECT.to_string(),
            Val::Enum(context.subject.clone()),
        ),
        (
            component_abi::decision_summary::OPERATION.to_string(),
            Val::String(context.operation.clone()),
        ),
        (
            component_abi::decision_summary::TARGET_SUMMARY.to_string(),
            Val::String(context.target_summary.clone()),
        ),
        (
            component_abi::decision_summary::DECISION_ID.to_string(),
            Val::String(context.decision_id.clone()),
        ),
        (
            component_abi::decision_summary::TRACE_ID.to_string(),
            Val::String(context.trace_id.clone()),
        ),
        (
            component_abi::decision_summary::ACTOR_PROCESS_IDENTITY.to_string(),
            Val::String(context.actor_process_identity.clone()),
        ),
    ])
}

pub(super) fn matched_rule_val(context: &plugin_system::FilePolicyReadContext) -> Val {
    let rule = &context.matched_rule;
    Val::Record(vec![
        (
            component_abi::file_policy_view::RULE_ID.to_string(),
            Val::String(rule.rule_id.clone()),
        ),
        (
            component_abi::file_policy_view::DECISION.to_string(),
            Val::String(rule.decision.clone()),
        ),
        (
            component_abi::file_policy_view::OPERATION.to_string(),
            Val::String(rule.operation.clone()),
        ),
        (
            component_abi::file_policy_view::PATH.to_string(),
            Val::String(rule.path.clone()),
        ),
        (
            component_abi::file_policy_view::PLUGIN_INSTANCE.to_string(),
            component_option_string(rule.plugin_instance.clone()),
        ),
        (
            component_abi::file_policy_view::TIMEOUT_MS.to_string(),
            component_option_u64(rule.timeout_ms),
        ),
        (
            component_abi::file_policy_view::CONCURRENCY_LIMIT.to_string(),
            component_option_u32(rule.concurrency_limit),
        ),
        (
            component_abi::file_policy_view::FALLBACK.to_string(),
            component_option_string(rule.fallback.clone()),
        ),
    ])
}

pub(super) fn component_option_u64(value: Option<u64>) -> Val {
    Val::Option(value.map(|value| Box::new(Val::U64(value))))
}

pub(super) fn component_option_u32(value: Option<u32>) -> Val {
    Val::Option(value.map(|value| Box::new(Val::U32(value))))
}

pub(super) fn validate_plugin_command_request(
    request: &PluginCommandRequest,
    limits: &crate::engine::WasmHostLimits,
) -> Result<(), PluginRuntimeError> {
    if request.argv.len() > limits.plugin_command_argv_max_count {
        return Err(PluginRuntimeError::new(
            "plugin_command",
            format!(
                "plugin command argv count exceeded {}",
                limits.plugin_command_argv_max_count
            ),
        ));
    }
    if let Some(arg_len) = request
        .argv
        .iter()
        .map(String::len)
        .find(|arg_len| *arg_len > limits.plugin_command_arg_max_bytes)
    {
        return Err(PluginRuntimeError::new(
            "plugin_command",
            format!(
                "plugin command argument exceeded {} bytes: {arg_len}",
                limits.plugin_command_arg_max_bytes
            ),
        ));
    }
    Ok(())
}

pub(super) fn control_context_snapshot(
    request: &ControlDecisionRequest,
) -> Option<crate::engine::ControlContextSnapshot> {
    Some(crate::engine::ControlContextSnapshot {
        context_ref: request.context_ref.clone()?,
        decision_id: request.decision_id.clone(),
        trace_id: request.trace_id.clone(),
        subject: request.subject.as_str().to_string(),
        operation: request.operation.clone(),
        target_summary: request.target_summary.clone(),
        actor_process_identity: request.actor_process_identity.summary(),
    })
}

pub(super) fn plugin_command_request_val(request: &PluginCommandRequest) -> Val {
    Val::Record(vec![(
        component_abi::plugin_command_request::ARGV.to_string(),
        Val::List(request.argv.iter().cloned().map(Val::String).collect()),
    )])
}

pub(super) fn decision_request_val(request: &ControlDecisionRequest) -> Val {
    Val::Record(vec![
        (
            component_abi::decision_request::DECISION_ID.to_string(),
            Val::String(request.decision_id.clone()),
        ),
        (
            component_abi::decision_request::TRACE_ID.to_string(),
            Val::String(request.trace_id.clone()),
        ),
        (
            component_abi::decision_request::TASK_ID.to_string(),
            Val::Option(None),
        ),
        (
            component_abi::decision_request::SUBJECT.to_string(),
            Val::Enum(request.subject.as_str().to_string()),
        ),
        (
            component_abi::decision_request::ACTOR_PROCESS_IDENTITY.to_string(),
            actor_process_identity_val(&request.actor_process_identity),
        ),
        (
            component_abi::decision_request::OPERATION.to_string(),
            Val::String(request.operation.clone()),
        ),
        (
            component_abi::decision_request::TARGET_SUMMARY.to_string(),
            Val::String(request.target_summary.clone()),
        ),
        (
            component_abi::decision_request::CONTEXT_REF.to_string(),
            component_option_string(request.context_ref.clone()),
        ),
    ])
}

pub(super) fn actor_process_identity_val(
    actor: &plugin_system::ControlActorProcessIdentity,
) -> Val {
    Val::Record(vec![
        (
            component_abi::actor_process::PID.to_string(),
            Val::U32(actor.pid),
        ),
        (
            component_abi::actor_process::TASK_ID.to_string(),
            Val::Option(actor.task_id.map(|task_id| Box::new(Val::U32(task_id)))),
        ),
        (
            component_abi::actor_process::GENERATION.to_string(),
            Val::U64(actor.generation),
        ),
        (
            component_abi::actor_process::NAMESPACE.to_string(),
            component_option_string(actor.namespace.clone()),
        ),
    ])
}

pub(super) fn component_option_string(value: Option<String>) -> Val {
    Val::Option(value.map(|value| Box::new(Val::String(value))))
}

pub(super) fn parse_component_option_string_val(value: &Val) -> Result<Option<String>, String> {
    match value {
        Val::Option(Some(value)) => match value.as_ref() {
            Val::String(value) => Ok(Some(value.clone())),
            other => Err(format!("option must contain string, got {other:?}")),
        },
        Val::Option(None) => Ok(None),
        other => Err(format!("value must be option<string>, got {other:?}")),
    }
}

pub(super) fn component_field<'a>(
    fields: &'a [(String, Val)],
    name: &str,
) -> Result<&'a Val, String> {
    fields
        .iter()
        .find(|(field, _)| field == name)
        .map(|(_, value)| value)
        .ok_or_else(|| format!("missing field {name}"))
}

pub(super) fn component_field_enum<'a>(
    fields: &'a [(String, Val)],
    name: &str,
) -> Result<&'a str, String> {
    match component_field(fields, name)? {
        Val::Enum(value) => Ok(value),
        other => Err(format!("field {name} must be enum, got {other:?}")),
    }
}

pub(super) fn component_field_string<'a>(
    fields: &'a [(String, Val)],
    name: &str,
) -> Result<&'a str, String> {
    match component_field(fields, name)? {
        Val::String(value) => Ok(value),
        other => Err(format!("field {name} must be string, got {other:?}")),
    }
}

pub(super) fn component_field_u64(fields: &[(String, Val)], name: &str) -> Result<u64, String> {
    match component_field(fields, name)? {
        Val::U64(value) => Ok(*value),
        other => Err(format!("field {name} must be u64, got {other:?}")),
    }
}

pub(super) fn component_field_s32(fields: &[(String, Val)], name: &str) -> Result<i32, String> {
    match component_field(fields, name)? {
        Val::S32(value) => Ok(*value),
        other => Err(format!("field {name} must be s32, got {other:?}")),
    }
}

pub(super) fn component_field_list<'a>(
    fields: &'a [(String, Val)],
    name: &str,
) -> Result<&'a [Val], String> {
    match component_field(fields, name)? {
        Val::List(values) => Ok(values),
        other => Err(format!("field {name} must be list, got {other:?}")),
    }
}

pub(super) fn component_field_option<'a>(
    fields: &'a [(String, Val)],
    name: &str,
) -> Result<Option<&'a Val>, String> {
    match component_field(fields, name)? {
        Val::Option(Some(value)) => Ok(Some(value)),
        Val::Option(None) => Ok(None),
        other => Err(format!("field {name} must be option, got {other:?}")),
    }
}

pub(super) fn component_field_option_string<'a>(
    fields: &'a [(String, Val)],
    name: &str,
) -> Result<Option<&'a str>, String> {
    match component_field_option(fields, name)? {
        Some(Val::String(value)) => Ok(Some(value)),
        Some(other) => Err(format!(
            "field {name} option must contain string, got {other:?}"
        )),
        None => Ok(None),
    }
}

pub(super) fn component_field_option_u64(
    fields: &[(String, Val)],
    name: &str,
) -> Result<Option<u64>, String> {
    match component_field_option(fields, name)? {
        Some(Val::U64(value)) => Ok(Some(*value)),
        Some(other) => Err(format!(
            "field {name} option must contain u64, got {other:?}"
        )),
        None => Ok(None),
    }
}

pub(super) fn parse_decision_response(
    value: Val,
) -> Result<ControlDecisionResponse, PluginRuntimeError> {
    let response = match value {
        Val::Result(Ok(Some(ok))) => *ok,
        Val::Result(Ok(None)) => {
            return Err(PluginRuntimeError::new(
                "wasm_runtime",
                "wasm component decide returned ok without decision-response",
            ));
        }
        Val::Result(Err(Some(error))) => {
            let message = match *error {
                Val::String(message) => message,
                other => format!("{other:?}"),
            };
            return Err(PluginRuntimeError::new(
                "wasm_runtime",
                format!("wasm component decide returned error: {message}"),
            ));
        }
        Val::Result(Err(None)) => {
            return Err(PluginRuntimeError::new(
                "wasm_runtime",
                "wasm component decide returned error without message",
            ));
        }
        other => {
            return Err(PluginRuntimeError::new(
                "wasm_runtime",
                format!("wasm component decide returned invalid result {other:?}"),
            ));
        }
    };
    let fields = match response {
        Val::Record(fields) => fields,
        other => {
            return Err(PluginRuntimeError::new(
                "wasm_runtime",
                format!("wasm component decide returned invalid response {other:?}"),
            ));
        }
    };
    let verdict = match decision_field_enum(&fields, "verdict")?.as_str() {
        "allow" => ControlVerdict::Allow,
        "deny" => ControlVerdict::Deny,
        other => {
            return Err(PluginRuntimeError::new(
                "wasm_runtime",
                format!("wasm component decide returned unsupported verdict {other}"),
            ));
        }
    };
    let scope = match decision_field_enum(&fields, "scope")?.as_str() {
        "once" => DecisionScope::Once,
        "reusable" => DecisionScope::Reusable,
        other => {
            return Err(PluginRuntimeError::new(
                "wasm_runtime",
                format!("wasm component decide returned unsupported scope {other}"),
            ));
        }
    };
    let reason_code = decision_field_option_string(&fields, "reason-code")?;
    let reason_message = decision_field_option_string(&fields, "reason-message")?;
    Ok(ControlDecisionResponse {
        verdict,
        scope,
        reason: reason_message.or(reason_code),
    })
}

pub(super) fn parse_plugin_command_response(
    value: Val,
) -> Result<PluginCommandResponse, PluginRuntimeError> {
    let response = match value {
        Val::Result(Ok(Some(ok))) => *ok,
        Val::Result(Ok(None)) => {
            return Err(PluginRuntimeError::new(
                "wasm_runtime",
                "wasm component command returned ok without plugin-command-result",
            ));
        }
        Val::Result(Err(Some(error))) => {
            let message = match *error {
                Val::String(message) => message,
                other => format!("{other:?}"),
            };
            return Err(PluginRuntimeError::new(
                "plugin_command",
                format!("wasm component command returned error: {message}"),
            ));
        }
        Val::Result(Err(None)) => {
            return Err(PluginRuntimeError::new(
                "plugin_command",
                "wasm component command returned error without message",
            ));
        }
        other => {
            return Err(PluginRuntimeError::new(
                "wasm_runtime",
                format!("wasm component command returned invalid result {other:?}"),
            ));
        }
    };
    let fields = match response {
        Val::Record(fields) => fields,
        other => {
            return Err(PluginRuntimeError::new(
                "wasm_runtime",
                format!("wasm component command returned invalid response {other:?}"),
            ));
        }
    };
    Ok(PluginCommandResponse {
        exit_code: component_field_s32(&fields, component_abi::plugin_command_result::EXIT_CODE)
            .map_err(|error| PluginRuntimeError::new("wasm_runtime", error))?,
        stdout: component_field_string(&fields, component_abi::plugin_command_result::STDOUT)
            .map(str::to_string)
            .map_err(|error| PluginRuntimeError::new("wasm_runtime", error))?,
        stderr: component_field_string(&fields, component_abi::plugin_command_result::STDERR)
            .map(str::to_string)
            .map_err(|error| PluginRuntimeError::new("wasm_runtime", error))?,
    })
}

pub(super) fn decision_field_enum(
    fields: &[(String, Val)],
    name: &str,
) -> Result<String, PluginRuntimeError> {
    match fields.iter().find(|(field, _)| field == name) {
        Some((_, Val::Enum(value))) => Ok(value.clone()),
        Some((_, other)) => Err(PluginRuntimeError::new(
            "wasm_runtime",
            format!("wasm component decision field {name} has invalid value {other:?}"),
        )),
        None => Err(PluginRuntimeError::new(
            "wasm_runtime",
            format!("wasm component decision missing field {name}"),
        )),
    }
}

pub(super) fn decision_field_option_string(
    fields: &[(String, Val)],
    name: &str,
) -> Result<Option<String>, PluginRuntimeError> {
    match fields.iter().find(|(field, _)| field == name) {
        Some((_, Val::Option(Some(value)))) => match value.as_ref() {
            Val::String(value) => Ok(Some(value.clone())),
            other => Err(PluginRuntimeError::new(
                "wasm_runtime",
                format!("wasm component decision field {name} has invalid value {other:?}"),
            )),
        },
        Some((_, Val::Option(None))) => Ok(None),
        Some((_, other)) => Err(PluginRuntimeError::new(
            "wasm_runtime",
            format!("wasm component decision field {name} has invalid value {other:?}"),
        )),
        None => Err(PluginRuntimeError::new(
            "wasm_runtime",
            format!("wasm component decision missing field {name}"),
        )),
    }
}
