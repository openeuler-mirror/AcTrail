#![no_std]

extern crate alloc;

use alloc::alloc::{Layout, alloc, realloc};
use alloc::string::{String, ToString};
use alloc::vec;

#[global_allocator]
static ALLOCATOR: wee_alloc::WeeAlloc = wee_alloc::WeeAlloc::INIT;

wit_bindgen::generate!({
    path: "../../../../crates/core/plugin_system/wit",
    world: "control-plugin",
});

use actrail::plugin::types::{
    ControlSubject, ControlVerdict, DecisionScope, FilePolicyApplyMode, FilePolicyApplyRequest,
    FilePolicyApplyStatus, FilePolicyDecision, FilePolicyListFilter, FilePolicyMatchDryRunRequest,
    FilePolicyOperation, FilePolicyPatchItem, FilePolicyPatchOp, FilePolicyRuleDraft,
};
use actrail_plugin_abi::control as control_abi;
use exports::actrail::plugin::control_decider::{
    DecisionRequest, DecisionResponse, Guest as ControlGuest,
};

const OPERATION_OPEN: &str = "open";
const MATCHED_RULE_ID: &str = "gray-file";
const MATCHED_RULE_DECISION_GRAY: &str = "gray";
const MATCHED_RULE_FALLBACK_DENY: &str = "deny";
const MATCHED_RULE_TIMEOUT_MS: u64 = 5000;
const MATCHED_RULE_CONCURRENCY_LIMIT: u32 = 1;
const MERGE_ALLOW_HIGH_RULE_ID: &str = "merge-allow-high";
const MERGE_DENY_LOW_RULE_ID: &str = "merge-deny-low";
const MERGE_DENY_SAME_RULE_ID: &str = "merge-deny-same";
const MERGE_SHARED_FILE_NAME: &str = "shared.txt";

struct Component;

impl ControlGuest for Component {
    fn decide(request: DecisionRequest) -> Result<DecisionResponse, String> {
        wit_bindgen::rt::maybe_link_cabi_realloc();
        if !matches!(request.subject, ControlSubject::FileAccess) {
            return Err("expected file-access decision".to_string());
        }
        if !str_eq(&request.operation, OPERATION_OPEN) {
            return Err("expected open operation".to_string());
        }
        let summary = actrail::plugin::host::query_context(
            control_abi::context::CURRENT_DECISION,
            control_abi::query::DECISION_SUMMARY,
        )?;
        if !matches!(summary.subject, ControlSubject::FileAccess)
            || !str_eq(&summary.operation, OPERATION_OPEN)
            || summary.target_summary.is_empty()
            || summary.decision_id.is_empty()
        {
            return Err("decision summary missing expected fields".to_string());
        }
        let policy = actrail::plugin::host::file_access_current_match_get(
            control_abi::context::CURRENT_FILE_POLICY,
            control_abi::query::MATCHED_RULE,
        )?;
        if !is_supported_rule_id(&policy.rule_id)
            || !str_eq(&policy.decision, MATCHED_RULE_DECISION_GRAY)
            || !str_eq(
                policy.fallback.as_deref().unwrap_or_default(),
                MATCHED_RULE_FALLBACK_DENY,
            )
            || policy.timeout_ms != Some(MATCHED_RULE_TIMEOUT_MS)
            || policy.concurrency_limit != Some(MATCHED_RULE_CONCURRENCY_LIMIT)
            || !str_eq(&policy.operation, OPERATION_OPEN)
        {
            return Err("matched rule view missing expected fields".to_string());
        }
        let config = PolicyConfig::from_rule(&policy.rule_id, &request.target_summary)?;
        let revision = actrail::plugin::host::file_policy_rules_version_get()?;
        let apply = FilePolicyApplyRequest {
            base_revision: revision,
            mutation_id: "component-hostcalls-policy-sync".to_string(),
            reason: Some("allow current gray target after remote-style sync".to_string()),
            correlation_id: None,
            apply_mode: FilePolicyApplyMode::Partial,
            items: vec![FilePolicyPatchItem {
                op: FilePolicyPatchOp::Upsert,
                rule_id: None,
                rule: Some(FilePolicyRuleDraft {
                    rule_id: None,
                    decision: config.decision,
                    operation: FilePolicyOperation::Open,
                    path: config.path.clone(),
                    gray_target: None,
                    priority: config.priority,
                }),
            }],
        };
        let result = actrail::plugin::host::file_policy_rules_apply(&apply)?;
        if !matches!(result.status, FilePolicyApplyStatus::Accepted) || result.applied_count != 1 {
            return Err("file-policy-rules-apply did not accept current target".to_string());
        }
        let list = actrail::plugin::host::file_policy_rules_list(
            &FilePolicyListFilter {
                decision: Some(config.decision),
                path_prefix: Some(config.path.clone()),
                operation: Some(FilePolicyOperation::Open),
            },
            None,
            4,
        )?;
        if list.rules.is_empty()
            || list
                .rules
                .iter()
                .all(|rule| !str_eq(&rule.path, &config.path))
        {
            return Err("file-policy-rules-list did not return applied rule".to_string());
        }
        if config.verify_effective {
            let dry_run = actrail::plugin::host::file_policy_rules_match_dry_run(
                &FilePolicyMatchDryRunRequest {
                    path: config.path.clone(),
                    operation: FilePolicyOperation::Open,
                },
            )?;
            if !dry_run.matched
                || !decision_eq(dry_run.decision, config.decision)
                || dry_run.rule_id.is_none()
            {
                return Err("file-policy-rules-match-dry-run did not hit expected rule".to_string());
            }
        }
        Ok(DecisionResponse {
            verdict: ControlVerdict::Allow,
            scope: DecisionScope::Once,
            reason_code: Some("component-hostcalls-allow".to_string()),
            reason_message: Some("component hostcalls allowed gray file".to_string()),
        })
    }
}

struct PolicyConfig {
    decision: FilePolicyDecision,
    priority: i32,
    path: String,
    verify_effective: bool,
}

impl PolicyConfig {
    fn from_rule(rule_id: &str, default_path: &str) -> Result<Self, String> {
        if str_eq(rule_id, MATCHED_RULE_ID) {
            return Ok(Self {
                decision: FilePolicyDecision::Allow,
                priority: 10,
                path: default_path.to_string(),
                verify_effective: true,
            });
        }
        let path = sibling_path(default_path, MERGE_SHARED_FILE_NAME)?;
        if str_eq(rule_id, MERGE_ALLOW_HIGH_RULE_ID) {
            return Ok(Self {
                decision: FilePolicyDecision::Allow,
                priority: 20,
                path,
                verify_effective: true,
            });
        }
        if str_eq(rule_id, MERGE_DENY_LOW_RULE_ID) {
            return Ok(Self {
                decision: FilePolicyDecision::Deny,
                priority: 10,
                path,
                verify_effective: false,
            });
        }
        if str_eq(rule_id, MERGE_DENY_SAME_RULE_ID) {
            return Ok(Self {
                decision: FilePolicyDecision::Deny,
                priority: 20,
                path,
                verify_effective: true,
            });
        }
        Err("unsupported file policy test rule id".to_string())
    }
}

fn is_supported_rule_id(rule_id: &str) -> bool {
    str_eq(rule_id, MATCHED_RULE_ID)
        || str_eq(rule_id, MERGE_ALLOW_HIGH_RULE_ID)
        || str_eq(rule_id, MERGE_DENY_LOW_RULE_ID)
        || str_eq(rule_id, MERGE_DENY_SAME_RULE_ID)
}

fn decision_eq(left: FilePolicyDecision, right: FilePolicyDecision) -> bool {
    matches!(
        (left, right),
        (FilePolicyDecision::Default, FilePolicyDecision::Default)
            | (FilePolicyDecision::Allow, FilePolicyDecision::Allow)
            | (FilePolicyDecision::Deny, FilePolicyDecision::Deny)
            | (FilePolicyDecision::Gray, FilePolicyDecision::Gray)
    )
}

fn sibling_path(path: &str, sibling_name: &str) -> Result<String, String> {
    let bytes = path.as_bytes();
    let mut slash = None;
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'/' {
            slash = Some(index);
        }
        index += 1;
    }
    let Some(slash) = slash else {
        return Err("target path has no parent directory".to_string());
    };
    let mut out = String::new();
    out.push_str(&path[..slash + 1]);
    out.push_str(sibling_name);
    Ok(out)
}

fn str_eq(left: &str, right: &str) -> bool {
    bytes_eq(left.as_bytes(), right.as_bytes())
}

fn bytes_eq(left: &[u8], right: &[u8]) -> bool {
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
    loop {}
}
