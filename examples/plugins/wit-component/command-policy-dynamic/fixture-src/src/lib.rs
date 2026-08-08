#![no_std]

extern crate alloc;

use alloc::alloc::{Layout, alloc, realloc};
use alloc::string::String;

#[global_allocator]
static ALLOCATOR: wee_alloc::WeeAlloc = wee_alloc::WeeAlloc::INIT;

wit_bindgen::generate!({
    path: "../../../../../crates/core/plugin_system/wit",
    world: "managed-control-plugin",
});

mod policy;

use actrail::plugin::types::ControlSubject;
use exports::actrail::plugin::control_decider::{
    DecisionRequest, DecisionResponse, Guest as ControlGuest,
};
use exports::actrail::plugin::management_command::{
    Guest as ManagementGuest, PluginCommandRequest, PluginCommandResult,
};
use exports::actrail::plugin::runtime_config::Guest as RuntimeConfigGuest;
use policy::PolicyManager;

struct Component;

impl ControlGuest for Component {
    fn decide(request: DecisionRequest) -> Result<DecisionResponse, String> {
        wit_bindgen::rt::maybe_link_cabi_realloc();
        let subject = match request.subject {
            ControlSubject::CommandExecution => "command-execution",
            ControlSubject::FileAccess => "file-access",
            ControlSubject::NetworkAction => "network-action",
        };
        Err(alloc::format!(
            "wasm.command-policy-dynamic only publishes command routes and cannot decide gray {subject} requests"
        ))
    }
}

impl ManagementGuest for Component {
    fn handle_command(request: PluginCommandRequest) -> Result<PluginCommandResult, String> {
        wit_bindgen::rt::maybe_link_cabi_realloc();
        match PolicyManager::handle_command(&request.argv) {
            Ok(stdout) => Ok(PluginCommandResult {
                exit_code: 0,
                stdout,
                stderr: String::new(),
            }),
            Err(message) => Ok(PluginCommandResult {
                exit_code: 2,
                stdout: String::new(),
                stderr: alloc::format!("{message}\n"),
            }),
        }
    }
}

impl RuntimeConfigGuest for Component {
    fn get() -> Result<String, String> {
        wit_bindgen::rt::maybe_link_cabi_realloc();
        PolicyManager::configuration_json()
    }

    fn validate(config_json: String) -> Result<alloc::vec::Vec<String>, String> {
        wit_bindgen::rt::maybe_link_cabi_realloc();
        PolicyManager::validate_configuration(&config_json)
    }

    fn submit(config_json: String) -> Result<(), String> {
        wit_bindgen::rt::maybe_link_cabi_realloc();
        PolicyManager::submit_configuration(&config_json)
    }
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
