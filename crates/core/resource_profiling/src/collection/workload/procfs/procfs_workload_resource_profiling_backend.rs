use std::collections::BTreeMap;
use std::time::Instant;

use model_core::process::ProcessIdentity;

use super::process_counter_reader::ProcessCounterReader;
use super::{ProcfsWorkloadProfilingConfig, ProcfsWorkloadSample, VerifiedProcessTarget};

const NANOS_PER_SECOND: u128 = 1_000_000_000;
const PERCENT_MILLIS_SCALE: u128 = 100_000;

/// Sampling state for one workload, bound to its stable root identity.
pub struct ProcfsWorkloadResourceProfilingBackend {
    config: ProcfsWorkloadProfilingConfig,
    root: ProcessIdentity,
    reader: ProcessCounterReader,
    previous_cpu: BTreeMap<ProcessIdentity, CpuBaseline>,
}

struct CpuBaseline {
    ticks: u64,
    sampled_at: Instant,
}

impl crate::ResourceProfilingBackend for ProcfsWorkloadResourceProfilingBackend {
    type Input = [VerifiedProcessTarget];
    type Sample = ProcfsWorkloadSample;

    fn sample(
        &mut self,
        targets: &Self::Input,
        context: crate::SamplingContext,
    ) -> Result<Option<Self::Sample>, String> {
        self.sample(targets, context.sampled_at)
    }
}

impl crate::WorkloadResourceProfilingBackend for ProcfsWorkloadResourceProfilingBackend {
    fn root_identity(&self) -> ProcessIdentity {
        self.root
    }
}

impl ProcfsWorkloadResourceProfilingBackend {
    pub fn new(
        root: ProcessIdentity,
        config: ProcfsWorkloadProfilingConfig,
    ) -> Result<Self, String> {
        Ok(Self {
            root,
            config,
            reader: ProcessCounterReader::new()?,
            previous_cpu: BTreeMap::new(),
        })
    }

    pub fn include_children(&self) -> bool {
        self.config.include_children()
    }

    /// Targets are a per-call snapshot from the identity owner, never a second registry.
    pub fn sample(
        &mut self,
        targets: &[VerifiedProcessTarget],
        sampled_at: Instant,
    ) -> Result<Option<ProcfsWorkloadSample>, String> {
        let mut sample = ProcfsWorkloadSample {
            cpu_percent_millis: Some(0),
            rss_kb: 0,
            virtual_memory_kb: 0,
            sampled_processes: 0,
            root_threads: None,
            root_comm: None,
        };
        for target in targets {
            if !self.config.include_children() && target.identity != self.root {
                continue;
            }
            let Some(counters) = self.reader.read(target)? else {
                continue;
            };
            let cpu = self.cpu_rate(target.identity, counters.cpu_ticks, sampled_at);
            sample.cpu_percent_millis = sample
                .cpu_percent_millis
                .zip(cpu)
                .map(|(total, current)| total.saturating_add(current));
            sample.rss_kb = sample.rss_kb.saturating_add(counters.rss_kb);
            sample.virtual_memory_kb = sample
                .virtual_memory_kb
                .saturating_add(counters.virtual_memory_kb);
            sample.sampled_processes += 1;
            if target.identity == self.root {
                sample.root_threads = Some(counters.threads);
                sample.root_comm = Some(counters.comm);
            }
        }
        self.previous_cpu
            .retain(|_, previous| previous.sampled_at == sampled_at);
        Ok((sample.sampled_processes != 0).then_some(sample))
    }

    fn cpu_rate(&mut self, identity: ProcessIdentity, ticks: u64, now: Instant) -> Option<u64> {
        let previous = self.previous_cpu.insert(
            identity,
            CpuBaseline {
                ticks,
                sampled_at: now,
            },
        )?;
        let elapsed = now.checked_duration_since(previous.sampled_at)?.as_nanos();
        if elapsed == 0 || ticks < previous.ticks {
            return None;
        }
        let numerator = u128::from(ticks - previous.ticks)
            .saturating_mul(NANOS_PER_SECOND)
            .saturating_mul(PERCENT_MILLIS_SCALE);
        let denominator = elapsed.saturating_mul(u128::from(self.reader.clock_ticks_per_second()));
        u64::try_from(numerator / denominator).ok()
    }
}
