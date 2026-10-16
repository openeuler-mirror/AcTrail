use crate::{
    ProfilingError, ResourceMetricsDrain, SamplingContext, SamplingSelection,
    WorkloadSamplingBackend, WorkloadTargets,
};
use std::time::{Duration, Instant, SystemTime};

pub struct WorkloadProfiler<B> {
    backend: B,
    interval: Duration,
    next_sample_at: Option<Instant>,
}
impl<B> WorkloadProfiler<B> {
    pub fn new(backend: B, interval_ms: u64, enabled: bool) -> Result<Self, ProfilingError> {
        let interval = Self::validate_interval(interval_ms, enabled)?;
        let next_sample_at = if enabled {
            Some(Instant::now().checked_add(interval).ok_or_else(|| {
                ProfilingError::new("resource_config", "workload interval exceeds clock range")
            })?)
        } else {
            None
        };
        Ok(Self {
            backend,
            interval,
            next_sample_at,
        })
    }
    pub(crate) fn validate_interval(
        interval_ms: u64,
        enabled: bool,
    ) -> Result<Duration, ProfilingError> {
        let interval = Duration::from_millis(interval_ms);
        if enabled && (interval.is_zero() || Instant::now().checked_add(interval).is_none()) {
            return Err(ProfilingError::new(
                "resource_config",
                "workload interval must be positive and representable",
            ));
        }
        Ok(interval)
    }
    pub fn backend(&self) -> &B {
        &self.backend
    }
    pub fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }
    pub fn drain_due<C: ?Sized>(
        &mut self,
        targets: &dyn WorkloadTargets,
        context: &mut C,
    ) -> Result<ResourceMetricsDrain, ProfilingError>
    where
        B: WorkloadSamplingBackend<C>,
    {
        self.backend.maintain();
        let Some(deadline) = self.next_sample_at else {
            return Ok(ResourceMetricsDrain::default());
        };
        let now = Instant::now();
        if now < deadline {
            return Ok(ResourceMetricsDrain::default());
        }
        self.next_sample_at =
            Some(now.checked_add(self.interval).ok_or_else(|| {
                ProfilingError::new("resource_clock", "sampling deadline overflow")
            })?);
        self.backend.collect(
            targets,
            context,
            SamplingContext {
                sampled_at: now,
                observed_at: SystemTime::now(),
            },
            &mut SamplingSelection::default(),
        )
    }
    pub fn poll_timeout(&self) -> Option<Duration> {
        self.next_sample_at
            .map(|next| next.saturating_duration_since(Instant::now()))
    }
}
