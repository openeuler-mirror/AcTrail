// Forward decision context and publish explicitly reusable executable verdicts.
#![no_std]
extern crate alloc;
use alloc::alloc::{Layout, alloc, realloc};
use alloc::borrow::ToOwned;
use alloc::string::{String, ToString};
#[global_allocator]
static ALLOCATOR: wee_alloc::WeeAlloc = wee_alloc::WeeAlloc::INIT;
wit_bindgen::generate!({
    path: "../../../../crates/core/plugin_system/wit",
    inline: "
        package actrail:test-security-service;
        world managed-http-fixture {
            import actrail:plugin/host@0.4.0;
            import actrail:plugin/http-host@0.4.0;
            export actrail:plugin/control-decider@0.4.0;
            export actrail:plugin/management-command@0.4.0;
        }
    ",
    world: "managed-http-fixture",
    generate_all,
});
use actrail::plugin::types::{
    CommandPolicyApplyRequest, CommandPolicyApplyStatus, CommandPolicyDecision,
    CommandPolicyPatchItem, CommandPolicyPatchOp, CommandPolicyRuleDraft, ControlSubject,
    ControlVerdict, DecisionScope,
};
use exports::actrail::plugin::control_decider::{DecisionRequest, DecisionResponse, Guest};
struct Component;
impl exports::actrail::plugin::management_command::Guest for Component {
    fn handle_command(
        request: actrail::plugin::types::PluginCommandRequest,
    ) -> Result<actrail::plugin::types::PluginCommandResult, String> {
        let endpoint = request.argv.first().ok_or("missing HTTP endpoint")?;
        let mode = request.argv.get(1).ok_or("missing response mode")?;
        let response =
            actrail::plugin::http_host::request(&actrail::plugin::http_host::HttpRequest {
                method: "PUT".into(),
                url: endpoint.clone(),
                headers: alloc::vec![("Content-Type".into(), "application/json".into())],
                body: serde_json::json!({"mode": mode}).to_string().into_bytes(),
            })?;
        Ok(actrail::plugin::types::PluginCommandResult {
            exit_code: 0,
            stdout: String::from_utf8(response.body).map_err(|error| error.to_string())?,
            stderr: String::new(),
        })
    }
}
impl Guest for Component {
    fn decide(request: DecisionRequest) -> Result<DecisionResponse, String> {
        let mut body = serde_json::json!({
            "decision_id": request.decision_id,
            "trace_id": request.trace_id,
            "pid": request.actor_process_identity.pid,
            "operation": request.operation,
            "target": request.target_summary,
            "generation": request.actor_process_identity.generation,
        });
        if request.subject == ControlSubject::CommandExecution {
            let context = actrail::plugin::host::command_execution_current_context_query(
                request
                    .context_ref
                    .as_deref()
                    .ok_or("missing command context")?,
                "command-execution.v1",
            )?;
            body["executable"] = context.resolved_path.into();
            body["argv"] = serde_json::json!(context.argv);
            body["execveat_dirfd"] = context.execveat_dirfd.into();
            body["execveat_flags"] = context.execveat_flags.into();
        }
        let config = actrail::plugin::host::read_config(0, 4096);
        if config.status != actrail::plugin::types::ConfigReadStatus::Ok || config.truncated {
            return Err("plugin configuration is unavailable or exceeds the read limit".into());
        }
        let config: serde_json::Value =
            serde_json::from_slice(&config.bytes).map_err(|e| e.to_string())?;
        let endpoint = config["endpoint"].as_str().ok_or("missing endpoint")?;
        let mut http_request = actrail::plugin::http_host::HttpRequest {
            method: "PUT".into(),
            url: alloc::format!("{endpoint}/ungranted"),
            headers: alloc::vec![
                ("Content-Type".into(), "application/json".into()),
                ("X-Decision-Client".into(), "wasm-fixture".into()),
            ],
            body: body.to_string().into_bytes(),
        };
        // Exercise URL authorization through the real host before the permitted request.
        if actrail::plugin::http_host::request(&http_request).is_ok() {
            return Err("host accepted an ungranted URL".into());
        }
        http_request.url = endpoint.into();
        let response = actrail::plugin::http_host::request(&http_request)?;
        if response.status != 202
            || !response.headers.iter().any(|(name, value)| {
                name.eq_ignore_ascii_case("x-decision-source") && value == "mock-service"
            })
        {
            return Err("unexpected analysis response".into());
        }
        let answer: serde_json::Value =
            serde_json::from_slice(&response.body).map_err(|e| e.to_string())?;
        let verdict = match answer["verdict"].as_str() {
            Some("allow") => ControlVerdict::Allow,
            Some("deny") => ControlVerdict::Deny,
            _ => return Err("security service returned no verdict".into()),
        };
        if answer["reuse"].as_str() == Some("executable") {
            Self::publish_executable(&body, verdict)?;
        }
        Ok(DecisionResponse {
            verdict,
            scope: DecisionScope::Once,
            reason_code: Some("security-service".into()),
            reason_message: answer["reason"].as_str().map(ToOwned::to_owned),
        })
    }
}
impl Component {
    // The service authorizes the complete executable scope. Publication completes
    // before this suspended exec resumes, so later calls use the kernel rule.
    fn publish_executable(
        context: &serde_json::Value,
        verdict: ControlVerdict,
    ) -> Result<(), String> {
        let executable = context["executable"].as_str().ok_or("missing executable")?;
        let rule_id = alloc::format!("reuse:{executable}");
        let request = CommandPolicyApplyRequest {
            base_revision: actrail::plugin::host::command_policy_rules_version_get()?,
            mutation_id: context["decision_id"]
                .as_str()
                .ok_or("missing decision ID")?
                .into(),
            reason: Some("security service authorized executable reuse".into()),
            items: alloc::vec![CommandPolicyPatchItem {
                op: CommandPolicyPatchOp::Upsert,
                rule_id: Some(rule_id.clone()),
                rule: Some(CommandPolicyRuleDraft {
                    rule_id: Some(rule_id),
                    decision: match verdict {
                        ControlVerdict::Allow => CommandPolicyDecision::Allow,
                        ControlVerdict::Deny => CommandPolicyDecision::Deny,
                    },
                    executable: executable.into(),
                    args: None,
                    gray_target: None,
                    priority: 1,
                }),
            }],
        };
        let result = actrail::plugin::host::command_policy_rules_apply(&request)?;
        if !matches!(result.status, CommandPolicyApplyStatus::Accepted) || result.applied_count != 1
        {
            return Err("reusable executable policy was not published".into());
        }
        Ok(())
    }
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
