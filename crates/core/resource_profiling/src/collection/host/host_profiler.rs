use std::time::{Duration, Instant};

use crate::model::host_resource::HostResourceSample;

use super::HostResourceProfilingBackend;
use crate::SamplingContext;

pub struct HostProfiler {
    backend: Box<dyn HostResourceProfilingBackend>,
    interval: Duration,
    next_sample: Instant,
}

impl HostProfiler {
    pub fn new(
        backend: Box<dyn HostResourceProfilingBackend>,
        interval_ms: u64,
    ) -> Result<Self, String> {
        let interval = Duration::from_millis(interval_ms);
        if interval.is_zero() || Instant::now().checked_add(interval).is_none() {
            return Err("host interval must be positive and representable".into());
        }
        Ok(Self {
            backend,
            interval,
            next_sample: Instant::now(),
        })
    }

    pub fn poll_timeout(&self, now: Instant) -> Duration {
        self.next_sample.saturating_duration_since(now)
    }

    pub fn sample_due(
        &mut self,
        context: SamplingContext,
    ) -> Result<Option<HostResourceSample>, String> {
        if context.sampled_at < self.next_sample {
            return Ok(None);
        }
        self.next_sample = context
            .sampled_at
            .checked_add(self.interval)
            .ok_or("host sampling deadline overflow")?;
        self.backend.sample(&(), context)
    }
}
