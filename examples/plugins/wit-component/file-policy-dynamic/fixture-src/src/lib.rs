#![no_std]

extern crate alloc;

use alloc::alloc::{Layout, alloc, realloc};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;

#[global_allocator]
static ALLOCATOR: wee_alloc::WeeAlloc = wee_alloc::WeeAlloc::INIT;

wit_bindgen::generate!({
    path: "../../../../../crates/core/plugin_system/wit",
    world: "managed-control-plugin",
});

use actrail::plugin::types::{
    ControlSubject, ControlVerdict, DecisionScope, FilePolicyApplyMode, FilePolicyApplyRequest,
    FilePolicyApplyStatus, FilePolicyDecision, FilePolicyListFilter, FilePolicyMatchDryRunRequest,
    FilePolicyOperation, FilePolicyPatchItem, FilePolicyPatchOp, FilePolicyRuleDraft,
};
use exports::actrail::plugin::control_decider::{
    DecisionRequest, DecisionResponse, Guest as ControlGuest,
};
use exports::actrail::plugin::management_command::{
    Guest as ManagementGuest, PluginCommandRequest, PluginCommandResult,
};

struct Component;

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

fn handle_command_impl(argv: &[String]) -> Result<String, String> {
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
    let mut priority = 10_i32;
    let mut gray_target = None;
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
        } else {
            return Err(usage());
        }
    }
    if matches!(decision, FilePolicyDecision::Gray) && gray_target.is_none() {
        return Err("gray upsert requires --gray-target <instance-index>".to_string());
    }
    let revision = actrail::plugin::host::file_policy_rules_version_get()?;
    let request = FilePolicyApplyRequest {
        base_revision: revision,
        mutation_id: "dynamic-policy-upsert".to_string(),
        reason: Some("dynamic plugin command".to_string()),
        correlation_id: None,
        apply_mode: FilePolicyApplyMode::Partial,
        items: vec![FilePolicyPatchItem {
            op: FilePolicyPatchOp::Upsert,
            rule_id: None,
            rule: Some(FilePolicyRuleDraft {
                rule_id: None,
                decision,
                operation: FilePolicyOperation::Open,
                path: path.clone(),
                gray_target,
                priority,
            }),
        }],
    };
    let result = actrail::plugin::host::file_policy_rules_apply(&request)?;
    if !matches!(result.status, FilePolicyApplyStatus::Accepted) || result.applied_count != 1 {
        return Err("host rejected file policy upsert".to_string());
    }
    Ok(format!(
        "accepted revision={} applied={} path={}\n",
        result.new_revision, result.applied_count, path
    ))
}

fn handle_rule_delete(argv: &[String]) -> Result<String, String> {
    if argv.len() != 3 {
        return Err(usage());
    }
    let revision = actrail::plugin::host::file_policy_rules_version_get()?;
    let request = FilePolicyApplyRequest {
        base_revision: revision,
        mutation_id: "dynamic-policy-delete".to_string(),
        reason: Some("dynamic plugin command".to_string()),
        correlation_id: None,
        apply_mode: FilePolicyApplyMode::Partial,
        items: vec![FilePolicyPatchItem {
            op: FilePolicyPatchOp::Delete,
            rule_id: Some(argv[2].clone()),
            rule: None,
        }],
    };
    let result = actrail::plugin::host::file_policy_rules_apply(&request)?;
    if !matches!(result.status, FilePolicyApplyStatus::Accepted) || result.applied_count != 1 {
        return Err("host rejected file policy delete".to_string());
    }
    Ok(format!(
        "accepted revision={} deleted={}\n",
        result.new_revision, argv[2]
    ))
}

fn handle_rule_list() -> Result<String, String> {
    let result = actrail::plugin::host::file_policy_rules_list(
        &FilePolicyListFilter {
            decision: None,
            path_prefix: None,
            operation: Some(FilePolicyOperation::Open),
        },
        None,
        64,
    )?;
    let mut out = format!("revision={}\n", result.source_revision);
    for rule in result.rules {
        out.push_str(&rule.rule_id);
        out.push(' ');
        out.push_str(decision_name(rule.decision));
        out.push(' ');
        out.push_str(&rule.path);
        out.push(' ');
        out.push_str("priority=");
        out.push_str(&rule.priority.to_string());
        out.push('\n');
    }
    if let Some(cursor) = result.next_cursor {
        out.push_str("next_cursor=");
        out.push_str(&cursor);
        out.push('\n');
    }
    Ok(out)
}

fn handle_rule_dry_run(argv: &[String]) -> Result<String, String> {
    if argv.len() != 3 {
        return Err(usage());
    }
    let result =
        actrail::plugin::host::file_policy_rules_match_dry_run(&FilePolicyMatchDryRunRequest {
            path: argv[2].clone(),
            operation: FilePolicyOperation::Open,
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

fn parse_decision(value: &str) -> Result<FilePolicyDecision, String> {
    if str_eq(value, "allow") {
        Ok(FilePolicyDecision::Allow)
    } else if str_eq(value, "deny") {
        Ok(FilePolicyDecision::Deny)
    } else if str_eq(value, "gray") {
        Ok(FilePolicyDecision::Gray)
    } else {
        Err("decision must be allow, deny, or gray".to_string())
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
    "usage: rule upsert <allow|deny|gray> <path> [--priority N] [--gray-target INDEX] | rule delete <rule-id> | rule list | rule dry-run <path>".to_string()
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
