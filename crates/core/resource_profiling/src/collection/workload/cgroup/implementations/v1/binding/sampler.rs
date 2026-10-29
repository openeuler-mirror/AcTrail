use super::handle::CgroupV1BindingHandle;
use crate::model::ResourceAccountingCoverage;
use crate::model::ResourceAccountingMethod;
use crate::model::ResourceMetrics;
use crate::model::cgroup_v1_binding::CgroupV1Binding;
use crate::{ResourceProfilingBackend, SamplingContext, WorkloadResourceProfilingBackend};
use model_core::process::ProcessIdentity;
use std::{collections::BTreeMap, time::Instant};
pub struct CgroupV1BindingSampler {
    root: ProcessIdentity,
    pub(crate) binding: CgroupV1BindingHandle,
    previous_cpu: Option<(u64, Instant)>,
}
impl CgroupV1BindingSampler {
    pub(crate) fn new(root: ProcessIdentity, binding: CgroupV1BindingHandle) -> Self {
        Self {
            root,
            binding,
            previous_cpu: None,
        }
    }
    fn number(text: String) -> Result<u64, String> {
        text.trim()
            .parse()
            .map_err(|e| format!("invalid v1 counter: {e}"))
    }
    pub(crate) fn payload(
        &mut self,
        binding: &CgroupV1Binding,
        context: SamplingContext,
    ) -> Result<ResourceMetrics, String> {
        let ns = Self::number(
            self.binding
                .cpu("cpuacct.usage")
                .map_err(|e| e.to_string())?,
        )?;
        let current = Self::number(
            self.binding
                .memory("memory.usage_in_bytes")
                .map_err(|e| e.to_string())?,
        )?;
        let previous = self.previous_cpu.replace((ns, context.sampled_at));
        let cpu = previous.and_then(|(last, at)| {
            let elapsed = context.sampled_at.checked_duration_since(at)?.as_nanos();
            if elapsed == 0 {
                return None;
            }
            let delta = ns.checked_sub(last)?;
            u64::try_from(u128::from(delta) * 100_000 / elapsed).ok()
        });
        let mut metadata = BTreeMap::from([
            ("cgroup_version".into(), "v1".into()),
            ("ownership".into(), "existing-read-only".into()),
            (
                "cpuacct_path".into(),
                binding.cpuacct.membership.display().to_string(),
            ),
            (
                "memory_path".into(),
                binding.memory.membership.display().to_string(),
            ),
            (
                "memory_usage_basis".into(),
                "memory.usage_in_bytes (kernel approximate charged memory)".into(),
            ),
            ("memory_peak_window".into(), "cgroup_lifetime".into()),
            (
                "memory_stat_basis".into(),
                "hierarchical total_rss/total_cache/total_swap".into(),
            ),
            ("cpu_baseline_ready".into(), cpu.is_some().to_string()),
            (
                "cpu_rate_basis".into(),
                "delta(cpuacct.usage)/monotonic_elapsed_ns".into(),
            ),
        ]);
        let peak = match self
            .binding
            .memory("memory.max_usage_in_bytes")
            .map_err(|e| e.to_string())
            .and_then(Self::number)
        {
            Ok(value) => Some(value),
            Err(error) => {
                metadata.insert("memory_peak_error".into(), error);
                None
            }
        };
        let mut payload = ResourceMetrics {
            scope: "cgroup".into(),
            subject: binding.trace_id.to_string(),
            accounting_method: ResourceAccountingMethod::CgroupV1,
            accounting_coverage: ResourceAccountingCoverage::BroaderThanTrace,
            cpu_percent_millis: cpu,
            cpu_usage_usec: Some(ns / 1000),
            memory_current_bytes: Some(current),
            memory_peak_bytes: peak,
            metadata,
            ..ResourceMetrics::default()
        };
        // v1 memory.stat RSS is charged anonymous memory, not process RSS.
        match self.binding.memory("memory.stat") {
            Ok(text) => {
                let values = text
                    .lines()
                    .filter_map(|line| {
                        let mut f = line.split_whitespace();
                        Some((f.next()?, f.next()?.parse::<u64>().ok()?))
                    })
                    .collect::<BTreeMap<_, _>>();
                payload.memory_anon_bytes = values.get("total_rss").copied();
                payload.memory_file_bytes = values.get("total_cache").copied();
                payload.memory_swap_current_bytes = values.get("total_swap").copied();
            }
            Err(error) => {
                payload
                    .metadata
                    .insert("memory_stat_error".into(), error.to_string());
            }
        }
        Ok(payload)
    }
}
impl ResourceProfilingBackend for CgroupV1BindingSampler {
    type Input = CgroupV1Binding;
    type Sample = ResourceMetrics;
    fn sample(
        &mut self,
        input: &Self::Input,
        context: SamplingContext,
    ) -> Result<Option<Self::Sample>, String> {
        self.payload(input, context).map(Some)
    }
}
impl WorkloadResourceProfilingBackend for CgroupV1BindingSampler {
    fn root_identity(&self) -> ProcessIdentity {
        self.root
    }
}
impl crate::collection::workload::cgroup::CgroupResourceProfilingBackend
    for CgroupV1BindingSampler
{
}
