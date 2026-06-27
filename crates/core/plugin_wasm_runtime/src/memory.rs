use plugin_system::PluginRuntimeError;
use wasmtime::{Memory, TypedFunc};

use crate::engine::{WasmStore, call_error};

pub(crate) fn write_guest_bytes(
    store: &mut WasmStore,
    memory: Memory,
    alloc: TypedFunc<i32, i32>,
    bytes: &[u8],
) -> Result<(i32, i32), PluginRuntimeError> {
    let len = i32::try_from(bytes.len()).map_err(|error| {
        PluginRuntimeError::new(
            "wasm_runtime",
            format!("guest payload length overflow: {error}"),
        )
    })?;
    let ptr = alloc
        .call(&mut *store, len)
        .map_err(|error| call_error(store, "wasm alloc", error))?;
    if ptr < 0 {
        return Err(PluginRuntimeError::new(
            "wasm_runtime",
            format!("wasm alloc returned negative pointer {ptr}"),
        ));
    }
    memory
        .write(store, usize::try_from(ptr).unwrap_or_default(), bytes)
        .map_err(|error| {
            PluginRuntimeError::new("wasm_runtime", format!("write wasm memory failed: {error}"))
        })?;
    Ok((ptr, len))
}
