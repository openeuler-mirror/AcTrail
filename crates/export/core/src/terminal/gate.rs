use std::sync::{Mutex, MutexGuard};

use crate::{ExportError, ExportPublishResult};

use super::{TerminalActionLimits, TerminalActionRejection, ledger::TerminalActionLedger};

pub struct TerminalActionGate<R> {
    ledger: Mutex<TerminalActionLedger<R>>,
}

pub enum TerminalActionDelivery<R> {
    Attempted(ExportPublishResult),
    Skipped(TerminalActionRejection<R>),
}

impl<R: Copy + Eq> TerminalActionGate<R> {
    pub fn new(limits: TerminalActionLimits) -> Self {
        Self {
            ledger: Mutex::new(TerminalActionLedger::new(limits)),
        }
    }

    /// Prepare outside the lock, then atomically recheck, try admission, and record.
    /// `publish` must only attempt nonblocking queue admission; it must not encode
    /// or perform file/network I/O. A rejected queue admission remains retryable.
    pub fn deliver<M>(
        &self,
        trace_id: u64,
        id: &str,
        revision: R,
        prepare: impl FnOnce() -> Result<M, ExportError>,
        publish: impl FnOnce(M) -> Result<ExportPublishResult, ExportError>,
    ) -> Result<TerminalActionDelivery<R>, ExportError> {
        let rejection = self.lock()?.rejection(trace_id, id, revision);
        if let Some(rejection) = rejection {
            return Ok(TerminalActionDelivery::Skipped(rejection));
        }
        let message = prepare()?;
        let id = id.to_owned();
        let mut ledger = self.lock()?;
        if let Some(rejection) = ledger.rejection(trace_id, &id, revision) {
            return Ok(TerminalActionDelivery::Skipped(rejection));
        }
        let result = publish(message)?;
        if result.dropped_outcome().is_none() {
            ledger.record(trace_id, id, revision);
        }
        Ok(TerminalActionDelivery::Attempted(result))
    }

    /// Call after the last delivery for this trace; callers serialize this
    /// lifecycle boundary with observation delivery.
    pub fn finish_trace(&self, trace_id: u64) -> Result<(), ExportError> {
        let removed = self.lock()?.finish_trace(trace_id);
        drop(removed);
        Ok(())
    }

    pub fn clear(&self) -> Result<(), ExportError> {
        let removed = self.lock()?.clear();
        drop(removed);
        Ok(())
    }

    fn lock(&self) -> Result<MutexGuard<'_, TerminalActionLedger<R>>, ExportError> {
        self.ledger.lock().map_err(|error| {
            ExportError::new(
                "terminal_action_ledger",
                format!("terminal action ledger lock poisoned: {error}"),
            )
        })
    }
}
