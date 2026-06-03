//! rustls plaintext hook handlers.

use std::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};

use tls_payload_core::PayloadDirection;

use crate::runtime::config;
use crate::runtime::decision::{RuntimeAction, decide_payload};
use crate::runtime::{hook, maps, output};

const RUSTLS_BUFFER_PLAINTEXT_SYMBOL: &str = "rustls_buffer_plaintext";
const RUSTLS_TAKE_RECEIVED_PLAINTEXT_SYMBOL: &str = "rustls_take_received_plaintext";
const RUSTLS_INLINE_TAG: usize = 0;
const RUSTLS_BORROWED_TAG: usize = 0x8000_0000_0000_0000;

static RUSTLS_BUFFER_PLAINTEXT_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static RUSTLS_TAKE_RECEIVED_PLAINTEXT_ORIGINAL: AtomicUsize = AtomicUsize::new(0);

type BufferPlaintextFn =
    unsafe extern "C" fn(*mut c_void, *mut RustlsOutboundChunks, *mut c_void) -> usize;
type TakeReceivedPlaintextFn = unsafe extern "C" fn(*mut c_void, *mut RustlsPayload);

#[repr(C)]
struct RustlsPayload {
    tag: usize,
    pointer: *mut u8,
    length: usize,
}

#[repr(C)]
struct RustlsOutboundChunks {
    q0: usize,
    q1: usize,
    q2: usize,
    q3: usize,
}

#[repr(C)]
struct RustlsChunk {
    pointer: *mut u8,
    length: usize,
}

pub(super) fn can_handle(symbol: &str) -> bool {
    matches!(
        symbol,
        RUSTLS_BUFFER_PLAINTEXT_SYMBOL | RUSTLS_TAKE_RECEIVED_PLAINTEXT_SYMBOL
    )
}

pub(super) fn install(symbol: &str, address: usize) -> Result<usize, String> {
    let replacement = match symbol {
        RUSTLS_BUFFER_PLAINTEXT_SYMBOL => hook_rustls_buffer_plaintext as *const () as usize,
        RUSTLS_TAKE_RECEIVED_PLAINTEXT_SYMBOL => {
            hook_rustls_take_received_plaintext as *const () as usize
        }
        _ => return Err(format!("unsupported rustls hook symbol: {symbol}")),
    };
    let trampoline = hook::install(address, replacement)?;
    match symbol {
        RUSTLS_BUFFER_PLAINTEXT_SYMBOL => {
            RUSTLS_BUFFER_PLAINTEXT_ORIGINAL.store(trampoline, Ordering::Release);
        }
        RUSTLS_TAKE_RECEIVED_PLAINTEXT_SYMBOL => {
            RUSTLS_TAKE_RECEIVED_PLAINTEXT_ORIGINAL.store(trampoline, Ordering::Release);
        }
        _ => unreachable!(),
    }
    Ok(trampoline)
}

unsafe extern "C" fn hook_rustls_buffer_plaintext(
    state: *mut c_void,
    payload: *mut RustlsOutboundChunks,
    sendable_plaintext: *mut c_void,
) -> usize {
    let original = unsafe { original_buffer_plaintext() };
    if payload.is_null() {
        return unsafe { original(state, payload, sendable_plaintext) };
    }
    match rewrite_outbound_chunks(state as usize, payload) {
        RustlsDecision::Continue => unsafe { original(state, payload, sendable_plaintext) },
        RustlsDecision::Block => 0,
    }
}

unsafe extern "C" fn hook_rustls_take_received_plaintext(
    state: *mut c_void,
    payload: *mut RustlsPayload,
) {
    let original = unsafe { original_take_received_plaintext() };
    if !payload.is_null() {
        rewrite_received_payload(state as usize, payload);
    }
    unsafe {
        original(state, payload);
    }
}

enum RustlsDecision {
    Continue,
    Block,
}

fn rewrite_received_payload(stream_key: usize, payload: *mut RustlsPayload) {
    let payload = unsafe { &mut *payload };
    if payload.tag != RUSTLS_BORROWED_TAG {
        abort_runtime(&format!(
            "rustls inbound unsupported Payload tag=0x{:x}",
            payload.tag
        ));
    }
    if payload.pointer.is_null() || payload.length == 0 {
        return;
    }
    rewrite_slice_or_abort(
        PayloadDirection::Inbound,
        RUSTLS_TAKE_RECEIVED_PLAINTEXT_SYMBOL,
        stream_key,
        payload.pointer,
        payload.length,
    );
}

fn rewrite_outbound_chunks(
    stream_key: usize,
    payload: *mut RustlsOutboundChunks,
) -> RustlsDecision {
    let payload = unsafe { &mut *payload };
    if payload.q0 == RUSTLS_INLINE_TAG {
        return rewrite_outbound_slice(stream_key, payload.q1 as *mut u8, payload.q2);
    }
    rewrite_outbound_multiple(stream_key, payload)
}

fn rewrite_outbound_multiple(stream_key: usize, payload: &RustlsOutboundChunks) -> RustlsDecision {
    let chunks = payload.q0 as *const RustlsChunk;
    let chunk_count = payload.q1;
    let start = payload.q2;
    let end = payload.q3;
    if chunks.is_null() {
        abort_runtime("rustls outbound Multiple has null chunks pointer");
    }
    if start > end {
        abort_runtime("rustls outbound Multiple has start > end");
    }
    let Some(selected_len) = end.checked_sub(start) else {
        abort_runtime("rustls outbound Multiple selected length underflow");
    };
    if config::get().is_some_and(|config| selected_len > config.max_payload_bytes()) {
        if config::get().is_some_and(|config| config.should_print_decision()) {
            output::event_line(&format!(
                "sync_decision: block direction=outbound symbol={RUSTLS_BUFFER_PLAINTEXT_SYMBOL} reason=max_payload_bytes length={selected_len}\n"
            ));
        }
        return RustlsDecision::Block;
    }
    let mut cursor = 0usize;
    for index in 0..chunk_count {
        let chunk = unsafe { &*chunks.add(index) };
        let Some(chunk_end) = cursor.checked_add(chunk.length) else {
            abort_runtime("rustls outbound chunk cursor overflow");
        };
        let overlap_start = start.max(cursor);
        let overlap_end = end.min(chunk_end);
        if overlap_start < overlap_end {
            let offset = overlap_start - cursor;
            let length = overlap_end - overlap_start;
            if let RustlsDecision::Block =
                rewrite_outbound_slice(stream_key, unsafe { chunk.pointer.add(offset) }, length)
            {
                return RustlsDecision::Block;
            }
        }
        cursor = chunk_end;
        if cursor >= end {
            break;
        }
    }
    RustlsDecision::Continue
}

fn rewrite_outbound_slice(stream_key: usize, pointer: *mut u8, length: usize) -> RustlsDecision {
    if pointer.is_null() || length == 0 {
        return RustlsDecision::Continue;
    }
    let payload = unsafe { std::slice::from_raw_parts(pointer, length) };
    match decide_payload(
        PayloadDirection::Outbound,
        RUSTLS_BUFFER_PLAINTEXT_SYMBOL,
        stream_key,
        payload,
    ) {
        RuntimeAction::Allow => RustlsDecision::Continue,
        RuntimeAction::Replace(replacement) => {
            write_replacement_or_abort(
                RUSTLS_BUFFER_PLAINTEXT_SYMBOL,
                PayloadDirection::Outbound,
                pointer,
                &replacement,
            );
            RustlsDecision::Continue
        }
        RuntimeAction::Block => RustlsDecision::Block,
    }
}

fn rewrite_slice_or_abort(
    direction: PayloadDirection,
    symbol: &str,
    stream_key: usize,
    pointer: *mut u8,
    length: usize,
) {
    let payload = unsafe { std::slice::from_raw_parts(pointer, length) };
    match decide_payload(direction, symbol, stream_key, payload) {
        RuntimeAction::Allow => {}
        RuntimeAction::Replace(replacement) => {
            write_replacement_or_abort(symbol, direction, pointer, &replacement);
        }
        RuntimeAction::Block => abort_runtime("rustls inbound processor blocked payload"),
    }
}

fn write_replacement_or_abort(
    symbol: &str,
    direction: PayloadDirection,
    pointer: *mut u8,
    replacement: &[u8],
) {
    if !maps::is_writable_range(pointer as usize, replacement.len()) {
        abort_runtime(&format!(
            "rustls {} replacement target is not writable symbol={symbol} pointer=0x{:x} bytes={}",
            direction.as_str(),
            pointer as usize,
            replacement.len()
        ));
    }
    unsafe {
        std::ptr::copy_nonoverlapping(replacement.as_ptr(), pointer, replacement.len());
    }
}

unsafe fn original_buffer_plaintext() -> BufferPlaintextFn {
    let address = RUSTLS_BUFFER_PLAINTEXT_ORIGINAL.load(Ordering::Acquire);
    if address == 0 {
        abort_runtime("rustls buffer_plaintext original is not installed");
    }
    unsafe { std::mem::transmute(address) }
}

unsafe fn original_take_received_plaintext() -> TakeReceivedPlaintextFn {
    let address = RUSTLS_TAKE_RECEIVED_PLAINTEXT_ORIGINAL.load(Ordering::Acquire);
    if address == 0 {
        abort_runtime("rustls take_received_plaintext original is not installed");
    }
    unsafe { std::mem::transmute(address) }
}

fn abort_runtime(message: &str) -> ! {
    output::error_line(&format!("tls_payload_probe_sync abort: {message}\n"));
    unsafe {
        libc::_exit(126);
    }
}
