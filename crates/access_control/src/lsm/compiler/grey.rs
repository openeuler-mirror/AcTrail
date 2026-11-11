//! Encode the complete grey decision budget into the immutable policy value.
use super::PolicySnapshot;

impl PolicySnapshot {
    pub(super) fn encode_grey(
        value: &mut [u8; 432],
        target: &str,
        timeout_ms: u64,
        concurrency_limit: u32,
        fallback_deny: bool,
    ) -> Result<(), String> {
        if target.is_empty() || target.len() >= 128 || target.contains('\0') {
            return Err("LSM grey target must contain 1..127 bytes without NUL".into());
        }
        let timeout_ms = u32::try_from(timeout_ms)
            .map_err(|_| "LSM grey timeout_ms exceeds the kernel u32 limit")?;
        if timeout_ms == 0 || concurrency_limit == 0 {
            return Err("LSM grey timeout_ms and concurrency_limit must be positive".into());
        }
        value[288..288 + target.len()].copy_from_slice(target.as_bytes());
        for (offset, field) in [
            (416, timeout_ms),
            (420, concurrency_limit),
            (424, u32::from(fallback_deny)),
        ] {
            value[offset..offset + 4].copy_from_slice(&field.to_ne_bytes());
        }
        Ok(())
    }
}
