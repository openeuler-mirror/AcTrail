//! Forward pending operations to the proxy owned by this control runtime.
#![no_std]
extern crate alloc;
use alloc::alloc::{Layout, alloc, realloc};
use alloc::borrow::ToOwned;
use alloc::string::{String, ToString};
use alloc::{format, vec};
#[global_allocator]
static ALLOCATOR: wee_alloc::WeeAlloc = wee_alloc::WeeAlloc::INIT;
wit_bindgen::generate!({
    path: "../../core/plugin_system/wit",
    inline: "
        package actrail:grey-forwarding;
        world grey-forwarding {
            import actrail:plugin/host@0.4.0;
            import actrail:plugin/http-host@0.4.0;
            export actrail:plugin/control-decider@0.4.0;
        }
    ",
    world: "grey-forwarding",
    generate_all,
});
use actrail::plugin::types::{ConfigReadStatus, ControlSubject, ControlVerdict, DecisionScope};
use exports::actrail::plugin::control_decider::{DecisionRequest, DecisionResponse, Guest};
struct GreyForwarding;
impl Guest for GreyForwarding {
    fn decide(request: DecisionRequest) -> Result<DecisionResponse, String> {
        let mut body = serde_json::json!({
            "decision_id": request.decision_id, "trace_id": request.trace_id,
            "logical_process_id": request.logical_process_id,
            "pid": request.actor_process_identity.pid, "generation": request.actor_process_identity.generation,
            "operation": request.operation, "target": request.target_summary,
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
        // Configuration is provided by the runtime, including the child endpoint.
        let size = actrail::plugin::host::read_config(0, 0)
            .total_size_hint
            .ok_or("missing configuration size")?;
        let config = actrail::plugin::host::read_config(0, size);
        if config.status != ConfigReadStatus::Ok || config.truncated {
            return Err("grey forwarding configuration unavailable".into());
        }
        let config: serde_json::Value =
            serde_json::from_slice(&config.bytes).map_err(|e| e.to_string())?;
        let endpoint = config["endpoint"]
            .as_str()
            .ok_or("missing runtime proxy endpoint")?;
        let response =
            actrail::plugin::http_host::request(&actrail::plugin::http_host::HttpRequest {
                method: "PUT".into(),
                url: endpoint.into(),
                headers: vec![("Content-Type".into(), "application/json".into())],
                body: serde_json::to_vec(&body).map_err(|e| e.to_string())?,
            })?;
        if !(200..300).contains(&response.status) {
            return Err(format!("grey proxy HTTP {}", response.status));
        }
        let answer: serde_json::Value =
            serde_json::from_slice(&response.body).map_err(|e| e.to_string())?;
        Ok(DecisionResponse {
            verdict: match answer["verdict"].as_str() {
                Some("allow") => ControlVerdict::Allow,
                Some("deny") => ControlVerdict::Deny,
                _ => return Err("external service returned no verdict".into()),
            },
            scope: DecisionScope::Once,
            reason_code: Some("grey-proxy".into()),
            reason_message: answer["reason"].as_str().map(str::to_owned),
        })
    }
}
export!(GreyForwarding);

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
