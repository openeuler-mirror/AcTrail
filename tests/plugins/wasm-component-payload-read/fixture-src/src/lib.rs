#![no_std]

extern crate alloc;

use alloc::alloc::{Layout, alloc, realloc};
use alloc::string::{String, ToString};

#[global_allocator]
static ALLOCATOR: wee_alloc::WeeAlloc = wee_alloc::WeeAlloc::INIT;

wit_bindgen::generate!({
    path: "../../../../crates/core/plugin_system/wit",
    world: "observation-plugin",
});

use exports::actrail::plugin::observation_consumer::{Guest, ObservationBatch, ObservationReport};

struct Component;

impl Guest for Component {
    fn consume(batch: ObservationBatch) -> Result<ObservationReport, String> {
        wit_bindgen::rt::maybe_link_cabi_realloc();
        for payload_ref in batch.payload_refs {
            let chunk = actrail::plugin::host::read_payload(&payload_ref, 1, 3);
            if chunk.status == actrail::plugin::types::PayloadReadStatus::Truncated
                && chunk.bytes.len() == 3
                && chunk.bytes[0] == b'O'
                && chunk.bytes[1] == b'S'
                && chunk.bytes[2] == b'T'
                && chunk.offset == 1
                && chunk.next_offset.is_some()
            {
                return Ok(ObservationReport {
                    observed_records: 1,
                    dropped_records: 0,
                });
            }
        }
        Err("payload-read did not return POST offset chunk".to_string())
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
