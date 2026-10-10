use super::*;
use access_control::lsm::{LsmAuditEvent, LsmController};
use model_core::process::HostProcessCoordinates;

impl EbpfCollector {
    pub fn set_lsm_controller(&mut self, controller: Arc<LsmController>) {
        self.loader.set_lsm_controller(controller);
    }

    pub fn lsm_poll_fd(&self) -> Option<i32> {
        self.runtime.as_ref()?.lsm.as_ref().map(|lsm| lsm.poll_fd())
    }

    pub fn drain_lsm_events(&self) -> Result<Vec<LsmAuditEvent>, String> {
        match self
            .runtime
            .as_ref()
            .and_then(|runtime| runtime.lsm.as_ref())
        {
            Some(lsm) => lsm.drain(),
            None => Ok(Vec::new()),
        }
    }

    pub fn resolve_lsm_observation(
        &self,
        event: &LsmAuditEvent,
    ) -> Result<ProcessObservation, String> {
        if event.observer_tgid == 0 || event.start_boottime_ns == 0 {
            return Err("LSM audit is missing observer process coordinates".into());
        }
        if let Some(observation) = self.bindings.tracked_event_observation(
            TraceId::new(event.trace_id),
            event.kernel_tgid,
            event.start_boottime_ns,
        ) {
            return Ok(observation.clone());
        }
        let ticks = self
            .clock_ticks_per_second
            .ok_or("process clock frequency unavailable")?;
        let start_ticks =
            (u128::from(event.start_boottime_ns) * u128::from(ticks) / 1_000_000_000) as u64;
        Ok(ProcessObservation::host(
            HostProcessCoordinates::new(event.observer_tgid, start_ticks)
                .with_start_boottime_ns(event.start_boottime_ns),
        ))
    }

    pub fn lsm_observed_at(&self, event: &LsmAuditEvent) -> SystemTime {
        decode::wall_from_ktime(event.observed_ktime_ns)
    }
}
