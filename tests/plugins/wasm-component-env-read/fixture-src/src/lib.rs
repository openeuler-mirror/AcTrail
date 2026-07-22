#![no_std]

extern crate alloc;

use alloc::alloc::{Layout, alloc, realloc};
use alloc::string::{String, ToString};

#[global_allocator]
static ALLOCATOR: wee_alloc::WeeAlloc = wee_alloc::WeeAlloc::INIT;

const ENV_NAME: &str = "ACTRAIL_COMPONENT_ENV_SECRET";
const ENV_VALUE: &str = "component-secret";

wit_bindgen::generate!({
    path: "../../../../crates/core/plugin_system/wit",
    world: "observation-plugin",
});

use exports::actrail::plugin::observation_consumer::{Guest, ObservationBatch, ObservationReport};

struct Component;

impl Guest for Component {
    fn consume(_batch: ObservationBatch) -> Result<ObservationReport, String> {
        wit_bindgen::rt::maybe_link_cabi_realloc();
        let value = actrail::plugin::host::env_read(ENV_NAME, 64)?;
        if !bytes_equal(value.as_bytes(), ENV_VALUE.as_bytes()) {
            return Err("env-read returned unexpected value".to_string());
        }
        Ok(ObservationReport {
            observed_records: 1,
            dropped_records: 0,
        })
    }
}

fn bytes_equal(left: &[u8], right: &[u8]) -> bool {
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
