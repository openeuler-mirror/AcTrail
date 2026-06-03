//! SQLite-backed trace detail rendering (deprecated - use view module instead).

#[allow(dead_code)]
pub fn traces_json(_storage_path: &std::path::Path) -> Result<String, String> {
    // Deprecated - use crate::view::traces_json instead
    Ok("[]".to_string())
}

#[allow(dead_code)]
pub fn trace_json(_storage_path: &std::path::Path, _trace_id: u64) -> Result<String, String> {
    // Deprecated - use crate::view::trace_json instead
    Ok("{}".to_string())
}