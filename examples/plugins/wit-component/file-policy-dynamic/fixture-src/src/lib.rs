#![no_std]

extern crate alloc;

use alloc::alloc::{Layout, alloc, realloc};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use serde::{Deserialize, Serialize};
use spin::Mutex;

#[global_allocator]
static ALLOCATOR: wee_alloc::WeeAlloc = wee_alloc::WeeAlloc::INIT;

wit_bindgen::generate!({
    path: "../../../../../crates/core/plugin_system/wit",
    world: "managed-control-plugin",
});

use actrail::plugin::types::{
    ControlSubject, ControlVerdict, DecisionScope, FilePolicyApplyMode, FilePolicyApplyRequest,
    FilePolicyApplyStatus, FilePolicyDecision, FilePolicyMatchDryRunRequest, FilePolicyOperation,
    FilePolicyPatchItem, FilePolicyPatchOp, FilePolicyRuleDraft,
};
use exports::actrail::plugin::control_decider::{
    DecisionRequest, DecisionResponse, Guest as ControlGuest,
};
use exports::actrail::plugin::management_command::{
    Guest as ManagementGuest, PluginCommandRequest, PluginCommandResult,
};
use exports::actrail::plugin::runtime_config::Guest as RuntimeConfigGuest;

static POLICY_CONFIG: Mutex<Option<PolicyConfig>> = Mutex::new(None);

struct Component;

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
    #[serde(default)]
    operation: PolicyOperation,
    path: String,
    #[serde(default = "default_priority")]
    priority: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    gray_target: Option<u64>,
}

#[derive(Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
enum PolicyDecision {
    Allow,
    Deny,
    Gray,
}

#[derive(Clone, Copy, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
enum PolicyOperation {
    #[default]
    Any,
    Open,
    Mkdir,
    Rmdir,
}

impl ControlGuest for Component {
    fn decide(request: DecisionRequest) -> Result<DecisionResponse, String> {
        wit_bindgen::rt::maybe_link_cabi_realloc();
        if !matches!(request.subject, ControlSubject::FileAccess) {
            return Err("expected file-access decision".to_string());
        }
        let verdict = if path_hash_even(&request.target_summary) {
            ControlVerdict::Allow
        } else {
            ControlVerdict::Deny
        };
        Ok(DecisionResponse {
            verdict,
            scope: DecisionScope::Once,
            reason_code: Some("dynamic-hash".to_string()),
            reason_message: Some("file path hash parity decision".to_string()),
        })
    }
}

impl ManagementGuest for Component {
    fn handle_command(request: PluginCommandRequest) -> Result<PluginCommandResult, String> {
        wit_bindgen::rt::maybe_link_cabi_realloc();
        match handle_command_impl(&request.argv) {
            Ok(stdout) => Ok(PluginCommandResult {
                exit_code: 0,
                stdout,
                stderr: String::new(),
            }),
            Err(message) => Ok(PluginCommandResult {
                exit_code: 2,
                stdout: String::new(),
                stderr: format!("{message}\n"),
            }),
        }
    }
}

impl RuntimeConfigGuest for Component {
    fn get() -> Result<String, String> {
        wit_bindgen::rt::maybe_link_cabi_realloc();
        let config = current_config()?;
        serde_json::to_string(&config).map_err(|error| format!("serialize config: {error}"))
    }

    fn validate(config_json: String) -> Result<Vec<String>, String> {
        wit_bindgen::rt::maybe_link_cabi_realloc();
        let current = current_config()?;
        let candidate = match parse_and_normalize(&config_json, &current) {
            Ok(candidate) => candidate,
            Err(error) => return Ok(vec![error]),
        };
        if candidate == current {
            return Ok(Vec::new());
        }
        let request = projection_request(&current, &candidate, "web config validation")?;
        if request.items.is_empty() {
            return Ok(Vec::new());
        }
        let delete_count = current.rules.len() as u32;
        let result = actrail::plugin::host::file_policy_rules_validate(&request)?;
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

    fn submit(config_json: String) -> Result<(), String> {
        wit_bindgen::rt::maybe_link_cabi_realloc();
        let current = POLICY_CONFIG.lock().clone().unwrap_or_default();
        let candidate = parse_and_normalize(&config_json, &current)?;
        publish_and_commit(current, candidate, "plugin config submit")
    }
}

fn handle_command_impl(argv: &[String]) -> Result<String, String> {
    if argv.len() == 1 && (str_eq(&argv[0], "help") || str_eq(&argv[0], "--help")) {
        return Ok(help());
    }
    if argv.len() < 2 || !str_eq(&argv[0], "rule") {
        return Err(usage());
    }
    if str_eq(&argv[1], "upsert") {
        handle_rule_upsert(argv)
    } else if str_eq(&argv[1], "delete") {
        handle_rule_delete(argv)
    } else if str_eq(&argv[1], "list") {
        handle_rule_list()
    } else if str_eq(&argv[1], "dry-run") {
        handle_rule_dry_run(argv)
    } else {
        Err(usage())
    }
}

fn handle_rule_upsert(argv: &[String]) -> Result<String, String> {
    if argv.len() < 4 {
        return Err(usage());
    }
    let decision = parse_decision(&argv[2])?;
    let path = argv[3].clone();
    let mut priority = default_priority();
    let mut gray_target = None;
    let mut operation = PolicyOperation::Any;
    let mut index = 4;
    while index < argv.len() {
        if str_eq(&argv[index], "--priority") {
            let Some(value) = argv.get(index + 1) else {
                return Err("--priority requires a value".to_string());
            };
            priority = parse_i32(value)?;
            index += 2;
        } else if str_eq(&argv[index], "--gray-target") {
            let Some(value) = argv.get(index + 1) else {
                return Err("--gray-target requires a value".to_string());
            };
            gray_target = Some(parse_u64(value)?);
            index += 2;
        } else if str_eq(&argv[index], "--operation") {
            let Some(value) = argv.get(index + 1) else {
                return Err("--operation requires a value".to_string());
            };
            operation = parse_operation(value)?;
            index += 2;
        } else {
            return Err(usage());
        }
    }
    if matches!(decision, PolicyDecision::Gray) && gray_target.is_none() {
        return Err("gray upsert requires --gray-target <instance-index>".to_string());
    }
    let current = current_config()?;
    let mut candidate = current.clone();
    candidate.rules.push(PolicyRule {
        rule_id: None,
        decision,
        operation,
        path: path.clone(),
        priority,
        gray_target,
    });
    normalize_rule_ids(&mut candidate, &current)?;
    let rule_id = candidate
        .rules
        .last()
        .and_then(|rule| rule.rule_id.clone())
        .ok_or_else(|| "normalized rule has no id".to_string())?;
    publish_and_commit(current, candidate, "dynamic plugin command")?;
    Ok(format!("accepted rule_id={rule_id} path={path}\n"))
}

fn handle_rule_delete(argv: &[String]) -> Result<String, String> {
    if argv.len() != 3 {
        return Err(usage());
    }
    let current = current_config()?;
    let mut candidate = current.clone();
    let before = candidate.rules.len();
    candidate
        .rules
        .retain(|rule| rule.rule_id.as_deref() != Some(argv[2].as_str()));
    if candidate.rules.len() == before {
        return Err(format!("rule {} not found", argv[2]));
    }
    publish_and_commit(current, candidate, "dynamic plugin command")?;
    Ok(format!("accepted deleted={}\n", argv[2]))
}

fn handle_rule_list() -> Result<String, String> {
    let config = current_config()?;
    let mut out = String::new();
    for rule in config.rules {
        out.push_str(rule.rule_id.as_deref().unwrap_or("unassigned"));
        out.push(' ');
        out.push_str(rule.decision.as_str());
        out.push(' ');
        out.push_str(rule.operation.as_str());
        out.push(' ');
        out.push_str(&rule.path);
        out.push_str(" priority=");
        out.push_str(&rule.priority.to_string());
        if let Some(target) = rule.gray_target {
            out.push_str(" gray_target=");
            out.push_str(&target.to_string());
        }
        out.push('\n');
    }
    if out.is_empty() {
        out.push_str("no configured rules\n");
    }
    Ok(out)
}

fn handle_rule_dry_run(argv: &[String]) -> Result<String, String> {
    if argv.len() != 3 && argv.len() != 5 {
        return Err(usage());
    }
    let operation = if argv.len() == 5 && str_eq(&argv[3], "--operation") {
        parse_operation(&argv[4])?
    } else if argv.len() == 3 {
        PolicyOperation::Open
    } else {
        return Err(usage());
    };
    let result =
        actrail::plugin::host::file_policy_rules_match_dry_run(&FilePolicyMatchDryRunRequest {
            path: argv[2].clone(),
            operation: operation.host_value(),
        })?;
    Ok(format!(
        "matched={} decision={} rule_id={} path={} revision={}\n",
        result.matched,
        decision_name(result.decision),
        result.rule_id.unwrap_or_else(|| "none".to_string()),
        result.canonical_path,
        result.source_revision
    ))
}

fn current_config() -> Result<PolicyConfig, String> {
    POLICY_CONFIG
        .lock()
        .clone()
        .ok_or_else(|| "plugin runtime config has not been initialized".to_string())
}

fn parse_and_normalize(raw: &str, current: &PolicyConfig) -> Result<PolicyConfig, String> {
    let mut config = serde_json::from_str::<PolicyConfig>(raw)
        .map_err(|error| format!("parse config JSON: {error}"))?;
    normalize_rule_ids(&mut config, current)?;
    validate_config(&config)?;
    Ok(config)
}

fn normalize_rule_ids(config: &mut PolicyConfig, current: &PolicyConfig) -> Result<(), String> {
    let mut next_id = 1_u64;
    for rule in current.rules.iter().chain(config.rules.iter()) {
        if let Some(number) = rule
            .rule_id
            .as_deref()
            .and_then(|id| id.strip_prefix("dynamic-"))
            .and_then(|number| number.parse::<u64>().ok())
        {
            next_id = next_id.max(number.saturating_add(1));
        }
    }
    for rule in &mut config.rules {
        if rule
            .rule_id
            .as_deref()
            .is_none_or(|id| id.trim().is_empty())
        {
            rule.rule_id = Some(format!("dynamic-{next_id}"));
            next_id = next_id.saturating_add(1);
        }
    }
    validate_config(config)
}

fn validate_config(config: &PolicyConfig) -> Result<(), String> {
    for (index, rule) in config.rules.iter().enumerate() {
        let id = rule
            .rule_id
            .as_deref()
            .ok_or_else(|| format!("rules[{index}].rule_id is required"))?;
        if id.trim().is_empty() {
            return Err(format!("rules[{index}].rule_id must not be empty"));
        }
        if config.rules[..index]
            .iter()
            .any(|existing| existing.rule_id.as_deref() == Some(id))
        {
            return Err(format!("duplicate rule_id {id}"));
        }
        if rule.path.trim().is_empty() {
            return Err(format!("rules[{index}].path must not be empty"));
        }
        match rule.decision {
            PolicyDecision::Gray if rule.gray_target.is_none() => {
                return Err(format!("rules[{index}].gray_target is required for gray"));
            }
            PolicyDecision::Allow | PolicyDecision::Deny if rule.gray_target.is_some() => {
                return Err(format!("rules[{index}].gray_target is only valid for gray"));
            }
            _ => {}
        }
    }
    Ok(())
}

fn projection_request(
    current: &PolicyConfig,
    candidate: &PolicyConfig,
    reason: &str,
) -> Result<FilePolicyApplyRequest, String> {
    let revision = actrail::plugin::host::file_policy_rules_version_get()?;
    let mut items = Vec::with_capacity(current.rules.len() + candidate.rules.len());
    for rule in &current.rules {
        items.push(FilePolicyPatchItem {
            op: FilePolicyPatchOp::Delete,
            rule_id: rule.rule_id.clone(),
            rule: None,
        });
    }
    for rule in &candidate.rules {
        items.push(FilePolicyPatchItem {
            op: FilePolicyPatchOp::Upsert,
            rule_id: rule.rule_id.clone(),
            rule: Some(FilePolicyRuleDraft {
                rule_id: rule.rule_id.clone(),
                decision: rule.decision.host_value(),
                operation: rule.operation.host_value(),
                path: rule.path.clone(),
                gray_target: rule.gray_target,
                priority: rule.priority,
            }),
        });
    }
    Ok(FilePolicyApplyRequest {
        base_revision: revision,
        mutation_id: "dynamic-policy-config".to_string(),
        reason: Some(reason.to_string()),
        correlation_id: None,
        apply_mode: FilePolicyApplyMode::Partial,
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
    let request = projection_request(&current, &candidate, reason)?;
    if !request.items.is_empty() {
        let expected = request.items.len() as u32;
        let validation = actrail::plugin::host::file_policy_rules_validate(&request)?;
        if !validation.errors.is_empty() {
            let detail = validation
                .errors
                .first()
                .map(|error| error.message.as_str())
                .unwrap_or("host rejected file policy projection validation");
            return Err(detail.to_string());
        }
        let result = actrail::plugin::host::file_policy_rules_apply(&request)?;
        if !matches!(result.status, FilePolicyApplyStatus::Accepted)
            || result.applied_count != expected
        {
            let detail = result
                .errors
                .first()
                .map(|error| error.message.as_str())
                .unwrap_or("host rejected file policy projection");
            return Err(detail.to_string());
        }
    }
    *POLICY_CONFIG.lock() = Some(candidate);
    Ok(())
}

impl PolicyDecision {
    fn host_value(self) -> FilePolicyDecision {
        match self {
            Self::Allow => FilePolicyDecision::Allow,
            Self::Deny => FilePolicyDecision::Deny,
            Self::Gray => FilePolicyDecision::Gray,
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

impl PolicyOperation {
    fn host_value(self) -> FilePolicyOperation {
        match self {
            Self::Any => FilePolicyOperation::Any,
            Self::Open => FilePolicyOperation::Open,
            Self::Mkdir => FilePolicyOperation::Mkdir,
            Self::Rmdir => FilePolicyOperation::Rmdir,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Any => "any",
            Self::Open => "open",
            Self::Mkdir => "mkdir",
            Self::Rmdir => "rmdir",
        }
    }
}

fn parse_decision(value: &str) -> Result<PolicyDecision, String> {
    if str_eq(value, "allow") {
        Ok(PolicyDecision::Allow)
    } else if str_eq(value, "deny") {
        Ok(PolicyDecision::Deny)
    } else if str_eq(value, "gray") {
        Ok(PolicyDecision::Gray)
    } else {
        Err("decision must be allow, deny, or gray".to_string())
    }
}

fn parse_operation(value: &str) -> Result<PolicyOperation, String> {
    if str_eq(value, "any") {
        Ok(PolicyOperation::Any)
    } else if str_eq(value, "open") {
        Ok(PolicyOperation::Open)
    } else if str_eq(value, "mkdir") {
        Ok(PolicyOperation::Mkdir)
    } else if str_eq(value, "rmdir") {
        Ok(PolicyOperation::Rmdir)
    } else {
        Err("operation must be any, open, mkdir, or rmdir".to_string())
    }
}

fn decision_name(decision: FilePolicyDecision) -> &'static str {
    match decision {
        FilePolicyDecision::Default => "default",
        FilePolicyDecision::Allow => "allow",
        FilePolicyDecision::Deny => "deny",
        FilePolicyDecision::Gray => "gray",
    }
}

fn default_priority() -> i32 {
    10
}

fn parse_i32(value: &str) -> Result<i32, String> {
    value
        .parse::<i32>()
        .map_err(|_| "invalid i32 value".to_string())
}

fn parse_u64(value: &str) -> Result<u64, String> {
    value
        .parse::<u64>()
        .map_err(|_| "invalid u64 value".to_string())
}

fn path_hash_even(path: &str) -> bool {
    let mut hash = 0_u64;
    for byte in path.as_bytes() {
        hash = hash.wrapping_mul(131).wrapping_add(u64::from(*byte));
    }
    hash % 2 == 0
}

fn usage() -> String {
    "usage: help | rule upsert <allow|deny|gray> <path> [--operation any|open|mkdir|rmdir] [--priority N] [--gray-target INDEX] | rule delete <rule-id> | rule list | rule dry-run <path> [--operation open|mkdir|rmdir]".to_string()
}

fn help() -> String {
    "supported commands:\n  help\n  rule list\n  rule dry-run <path> [--operation open|mkdir|rmdir]\n  rule upsert <allow|deny|gray> <path> [--operation any|open|mkdir|rmdir] [--priority N] [--gray-target INDEX]\n  rule delete <rule-id>\n".to_string()
}

fn str_eq(left: &str, right: &str) -> bool {
    let left = left.as_bytes();
    let right = right.as_bytes();
    if left.len() != right.len() {
        return false;
    }
    let mut index = 0;
    while index < left.len() {
        if left[index] != right[index] {
            return false;
        }
        index += 1;
    }
    true
}

export!(Component);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn memcmp(left: *const u8, right: *const u8, len: usize) -> i32 {
    let mut index = 0;
    while index < len {
        let left_byte = unsafe { *left.add(index) };
        let right_byte = unsafe { *right.add(index) };
        if left_byte != right_byte {
            return i32::from(left_byte) - i32::from(right_byte);
        }
        index += 1;
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cabi_realloc(
    old_ptr: *mut u8,
    old_len: usize,
    align: usize,
    new_len: usize,
) -> *mut u8 {
    let layout;
    let ptr = unsafe {
        if old_len == 0 {
            if new_len == 0 {
                return align as *mut u8;
            }
            layout = Layout::from_size_align_unchecked(new_len, align);
            alloc(layout)
        } else {
            layout = Layout::from_size_align_unchecked(old_len, align);
            realloc(old_ptr, layout, new_len)
        }
    };
    if ptr.is_null() {
        core::arch::wasm32::unreachable();
    }
    ptr
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    core::arch::wasm32::unreachable();
}
