//! Manages queued idle-interval persistence operations.

use idle_contract::IdleStoreOp;

use super::IdleDetector;

impl IdleDetector {
    pub fn has_pending_ops(&self) -> bool {
        !self.pending_ops.is_empty()
    }

    /// Drains queued interval operations.
    pub fn drain_pending_ops(&mut self) -> Vec<IdleStoreOp> {
        std::mem::take(&mut self.pending_ops)
    }

    /// Requeues operations at the front after a persistence write failure, so they
    /// can be retried without being lost.
    pub fn restore_pending_ops(&mut self, mut ops: Vec<IdleStoreOp>) {
        ops.append(&mut self.pending_ops);
        self.pending_ops = ops;
    }
}
