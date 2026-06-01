//! Attach debug service implementation.

use control_contract::reply::ControlError;

use crate::service_host::AttachDebugService;

use super::SqliteAttachService;

impl AttachDebugService for SqliteAttachService {
    fn ebpf_debug_snapshot(
        &self,
        pid: u32,
    ) -> Result<ebpf_collector::EbpfCollectorDebugSnapshot, ControlError> {
        self.collector
            .debug_snapshot_for_pid(pid)
            .map_err(|error| ControlError::new(error.stage, error.message))
    }
}
