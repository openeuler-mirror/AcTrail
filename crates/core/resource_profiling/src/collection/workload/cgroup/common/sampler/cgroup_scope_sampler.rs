use super::process_rss::CgroupProcessRssReader;
use super::{CgroupCounterSource, CgroupProcessRssConfig, CgroupWorkloadSample};
use crate::{ResourceProfilingBackend, SamplingContext, WorkloadResourceProfilingBackend};
use model_core::process::ProcessIdentity;
use std::time::Instant;

/// Samples any `CgroupCounterSource`: managed v1/v2 scopes and v2 bindings.
pub struct CgroupScopeSampler<S> {
    root: ProcessIdentity,
    source: S,
    process_rss: CgroupProcessRssReader,
    previous_cpu: Option<(u64, Instant)>,
}

impl<S: CgroupCounterSource> CgroupScopeSampler<S> {
    pub fn new(
        root: ProcessIdentity,
        source: S,
        config: CgroupProcessRssConfig,
    ) -> Result<Self, String> {
        Ok(Self {
            root,
            source,
            process_rss: CgroupProcessRssReader::new(config)?,
            previous_cpu: None,
        })
    }
    pub fn source(&self) -> &S {
        &self.source
    }
    pub fn source_mut(&mut self) -> &mut S {
        &mut self.source
    }
}

impl<S: CgroupCounterSource> ResourceProfilingBackend for CgroupScopeSampler<S> {
    type Input = ();
    type Sample = CgroupWorkloadSample;

    fn sample(&mut self, _: &(), context: SamplingContext) -> Result<Option<Self::Sample>, String> {
        let counters = self
            .source
            .read_counters()
            .map_err(|error| error.to_string())?;
        let previous = self
            .previous_cpu
            .replace((counters.cpu.usage_usec, context.sampled_at));
        let rate = previous.and_then(|(usage, at)| {
            let elapsed = context.sampled_at.checked_duration_since(at)?.as_micros();
            if elapsed == 0 {
                return None;
            }
            let delta = counters.cpu.usage_usec.checked_sub(usage)?;
            let rate = u64::try_from(u128::from(delta) * 100_000 / elapsed).ok()?;
            Some((rate, elapsed))
        });
        let (process_rss, process_rss_error) = match self.process_rss.read(&self.source) {
            Ok(sample) => (sample, None),
            Err(error) => (None, Some(error)),
        };
        Ok(Some(CgroupWorkloadSample {
            counters,
            cpu_percent_millis: rate.map(|(rate, _)| rate),
            cpu_interval_usec: rate.map(|(_, interval)| interval),
            process_rss,
            process_rss_error,
        }))
    }
}

impl<S: CgroupCounterSource> WorkloadResourceProfilingBackend for CgroupScopeSampler<S> {
    fn root_identity(&self) -> ProcessIdentity {
        self.root
    }
}

impl<S: CgroupCounterSource> crate::collection::workload::cgroup::CgroupResourceProfilingBackend
    for CgroupScopeSampler<S>
{
}
