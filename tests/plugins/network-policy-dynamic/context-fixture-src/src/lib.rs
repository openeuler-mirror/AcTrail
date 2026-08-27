#![no_std]

extern crate alloc;

use alloc::alloc::{Layout, alloc, realloc};
use alloc::string::{String, ToString};

#[global_allocator]
static ALLOCATOR: wee_alloc::WeeAlloc = wee_alloc::WeeAlloc::INIT;

wit_bindgen::generate!({
    path: "../../../../crates/core/plugin_system/wit",
    world: "network-control-plugin",
});

use actrail::plugin::types::{ControlSubject, ControlVerdict, DecisionScope};
use actrail_plugin_abi::control as control_abi;
use exports::actrail::plugin::control_decider::{
    DecisionRequest, DecisionResponse, Guest as ControlGuest,
};

struct Component;

impl ControlGuest for Component {
    fn decide(request: DecisionRequest) -> Result<DecisionResponse, String> {
        wit_bindgen::rt::maybe_link_cabi_realloc();
        if !matches!(request.subject, ControlSubject::NetworkAction)
            || !str_eq(&request.operation, "connect")
        {
            return Err("expected network connect decision".to_string());
        }
        let context_ref = request
            .context_ref
            .ok_or_else(|| "network decision omitted context ref".to_string())?;
        let context = actrail::plugin::network_control_host::network_action_current_context_query(
            &context_ref,
            control_abi::query::NETWORK_ACTION_CONTEXT,
        )?;
        if !str_eq(&context.syscall, "connect")
            || !str_eq(&context.address_family, "ipv4")
            || !str_eq(&context.remote_address, "127.0.0.1")
            || context.remote_port == 0
            || context.fd < 3
            || context.ipv6_scope_id != 0
        {
            return Err("network action context has unexpected fields".to_string());
        }
        Ok(DecisionResponse {
            verdict: ControlVerdict::Deny,
            scope: DecisionScope::Reusable,
            reason_code: Some("typed-network-context-deny".to_string()),
            reason_message: Some("typed network context was verified".to_string()),
        })
    }
}

fn str_eq(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut index = 0;
    while index < left.len() {
        if left.as_bytes()[index] != right.as_bytes()[index] {
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
