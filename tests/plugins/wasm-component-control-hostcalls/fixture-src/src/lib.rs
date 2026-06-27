#![no_std]

extern crate alloc;

use alloc::alloc::{Layout, alloc, realloc};
use alloc::string::{String, ToString};

#[global_allocator]
static ALLOCATOR: wee_alloc::WeeAlloc = wee_alloc::WeeAlloc::INIT;

wit_bindgen::generate!({
    path: "../../../../crates/core/plugin_system/wit",
    world: "control-plugin",
});

use actrail::plugin::types::{
    ControlSubject, ControlVerdict, DecisionScope, FilePolicyUpdate, FilePolicyWriteStatus,
};
use actrail_plugin_abi::control as control_abi;
use exports::actrail::plugin::control_decider::{DecisionRequest, DecisionResponse, Guest};

const OPERATION_OPEN: &str = "open";
const MATCHED_RULE_ID: &str = "gray-file";
const MATCHED_RULE_DECISION_GRAY: &str = "gray";
const MATCHED_RULE_FALLBACK_DENY: &str = "deny";
const MATCHED_RULE_TIMEOUT_MS: u64 = 5000;
const MATCHED_RULE_CONCURRENCY_LIMIT: u32 = 1;
const LOCAL_RULE_ID: &str = "plugin-gray-allow";

struct Component;

impl Guest for Component {
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
        let policy = actrail::plugin::host::file_policy_read(
            control_abi::context::CURRENT_FILE_POLICY,
            control_abi::query::MATCHED_RULE,
        )?;
        if !str_eq(&policy.rule_id, MATCHED_RULE_ID)
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
        let update = FilePolicyUpdate {
            rule_id: LOCAL_RULE_ID.to_string(),
            decision: ControlVerdict::Allow,
            operation: OPERATION_OPEN.to_string(),
            path: request.target_summary,
        };
        let write_result = actrail::plugin::host::file_policy_write(
            control_abi::context::CURRENT_FILE_POLICY,
            &update,
        )?;
        if !matches!(write_result, FilePolicyWriteStatus::Accepted) {
            return Err("file-policy-write returned unexpected result".to_string());
        }
        Ok(DecisionResponse {
            verdict: ControlVerdict::Allow,
            scope: DecisionScope::Once,
            reason_code: Some("component-hostcalls-allow".to_string()),
            reason_message: Some("component hostcalls allowed gray file".to_string()),
        })
    }
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
