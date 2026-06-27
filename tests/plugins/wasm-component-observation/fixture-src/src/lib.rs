#![no_std]

extern crate alloc;

use alloc::alloc::{Layout, alloc, realloc};
use alloc::string::{String, ToString};
use core::sync::atomic::{AtomicU32, Ordering};

#[global_allocator]
static ALLOCATOR: wee_alloc::WeeAlloc = wee_alloc::WeeAlloc::INIT;

static CONFIG_OK: AtomicU32 = AtomicU32::new(0);
const EXPECTED_CONFIG: &[u8] = b"mode = \"component-config-ok\"\n";

wit_bindgen::generate!({
    path: "../../../../crates/core/plugin_system/wit",
    world: "observation-plugin",
});

use exports::actrail::plugin::observation_consumer::{Guest, ObservationBatch, ObservationReport};

struct Component;

impl Guest for Component {
    fn consume(_batch: ObservationBatch) -> Result<ObservationReport, String> {
        wit_bindgen::rt::maybe_link_cabi_realloc();
        if CONFIG_OK.load(Ordering::Relaxed) != 0 {
            return Ok(ObservationReport {
                observed_records: 1,
                dropped_records: 0,
            });
        }
        let chunk = actrail::plugin::host::read_config(0, 64);
        if chunk.status != actrail::plugin::types::ConfigReadStatus::Ok {
            return Err("read-config did not return ok".to_string());
        }
        if chunk.offset != 0 {
            return Err("read-config returned unexpected offset".to_string());
        }
        if chunk.truncated {
            return Err("read-config unexpectedly truncated config".to_string());
        }
        if chunk.next_offset.is_some() {
            return Err("read-config returned unexpected next offset".to_string());
        }
        if chunk.total_size_hint != Some(chunk.bytes.len() as u64) {
            return Err("read-config returned unexpected size hint".to_string());
        }
        if !matches_expected_config(&chunk.bytes) {
            return Err("read-config returned unexpected config bytes".to_string());
        }
        CONFIG_OK.store(1, Ordering::Relaxed);
        Ok(ObservationReport {
            observed_records: 1,
            dropped_records: 0,
        })
    }
}

fn matches_expected_config(config: &[u8]) -> bool {
    if config.len() != EXPECTED_CONFIG.len() {
        return false;
    }
    let mut index = 0;
    while index < EXPECTED_CONFIG.len() {
        if config[index] != EXPECTED_CONFIG[index] {
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
