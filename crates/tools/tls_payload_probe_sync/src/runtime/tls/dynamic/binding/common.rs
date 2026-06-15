use std::ffi::c_void;
use std::os::raw::c_char;

use crate::runtime::output;
use crate::runtime::tls::dynamic::core::{self, BindingSource, TlsFuncKind};

pub(in crate::runtime) fn maybe_bound_wrapper(
    symbol: *const c_char,
    real_sym: *mut c_void,
    source: BindingSource,
) -> *mut c_void {
    let Some(kind) = TlsFuncKind::from_c_symbol(symbol) else {
        return real_sym;
    };
    let real = real_sym as usize;
    match core::get_or_create_bound_wrapper(kind, real, source) {
        Some(wrapper) => wrapper as *mut c_void,
        None => {
            output::error_line(&format!(
                "tls_payload_probe_sync dynamic slot exhausted: symbol={} slots={}\n",
                kind.symbol(),
                core::SLOT_COUNT,
            ));
            real_sym
        }
    }
}
