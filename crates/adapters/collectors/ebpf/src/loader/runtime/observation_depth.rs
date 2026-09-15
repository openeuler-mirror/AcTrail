//! Sparse Agent-descendant observation depth control.

use super::*;

impl EbpfRuntime {
    pub(crate) fn set_process_observation_depth(
        &self,
        kernel_tgid: u32,
        start_boottime_ns: u64,
        remaining_depth: i32,
    ) -> Result<(), LoaderError> {
        if kernel_tgid == 0 || start_boottime_ns == 0 || remaining_depth < 0 {
            return Err(LoaderError::new(
                "process_observation_depth",
                format!(
                    "kernel TGID and generation must be non-zero and remaining depth non-negative, got {kernel_tgid}:{start_boottime_ns}:{remaining_depth}"
                ),
            ));
        }
        let mut scope = [0_u8; 16];
        scope[..8].copy_from_slice(&start_boottime_ns.to_ne_bytes());
        scope[8..12].copy_from_slice(&remaining_depth.to_ne_bytes());
        self.process_observation_depths
            .update(&kernel_tgid.to_ne_bytes(), &scope, MapFlags::ANY)
            .map_err(|error| LoaderError::new("process_observation_depth", error.to_string()))
    }

    pub(super) fn clear_process_observation_depth(
        &self,
        kernel_tgid: u32,
    ) -> Result<(), LoaderError> {
        let key = kernel_tgid.to_ne_bytes();
        let exists = self
            .process_observation_depths
            .lookup(&key, MapFlags::ANY)
            .map_err(|error| LoaderError::new("process_observation_depth", error.to_string()))?
            .is_some();
        if exists {
            self.process_observation_depths
                .delete(&key)
                .map_err(|error| {
                    LoaderError::new("process_observation_depth", error.to_string())
                })?;
        }
        Ok(())
    }
}
