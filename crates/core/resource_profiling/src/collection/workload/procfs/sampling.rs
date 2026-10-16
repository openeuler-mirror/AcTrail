use super::ProcfsSamplingSessions;
use crate::model::ResourceAccountingCoverage;
use crate::model::ResourceAccountingMethod;
use crate::model::ResourceMetrics;
use crate::model::ResourceSampleKind;
use crate::{
    ProfilingError, ResourceMetricsDrain, ResourceSampleDraft, ResourceSamplingFailure,
    SamplingContext, SamplingSelection, WorkloadSamplingBackend, WorkloadTargets,
};
use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;
use std::{
    collections::{BTreeMap, BTreeSet},
    time::{Instant, SystemTime},
};
const BYTES_PER_KIB: u64 = 1024;
impl<C: ?Sized> WorkloadSamplingBackend<C> for ProcfsSamplingSessions {
    fn collect(
        &mut self,
        targets: &dyn WorkloadTargets,
        _storage: &mut C,
        context: SamplingContext,
        selection: &mut SamplingSelection,
    ) -> Result<ResourceMetricsDrain, ProfilingError> {
        let mut drain = ResourceMetricsDrain::default();
        let mut active = BTreeSet::new();
        for trace in targets.traces() {
            if trace.terminal || !trace.requests_resources || selection.is_claimed(trace.trace_id) {
                continue;
            }
            selection.claim(trace.trace_id);
            active.insert(trace.trace_id);
            let processes = targets.process_targets(&trace, self.include_children());
            let metadata = selection.take_fallback_metadata(trace.trace_id);
            match self.collect_procfs_sample(
                trace.trace_id,
                trace.root,
                processes,
                targets,
                context.sampled_at,
                context.observed_at,
                false,
                metadata,
            ) {
                Ok(Some(draft)) => drain.samples.push(draft),
                Ok(None) => {}
                Err(error) => drain.failures.push(ResourceSamplingFailure {
                    trace_id: trace.trace_id,
                    message: format!("{}: {}", error.code, error.message),
                    recovered: false,
                }),
            }
        }
        self.retain(&active);
        Ok(drain)
    }
}
impl ProcfsSamplingSessions {
    #[allow(clippy::too_many_arguments)]
    fn collect_procfs_sample(
        &mut self,
        trace_id: TraceId,
        root: ProcessIdentity,
        processes: Vec<crate::VerifiedProcessTarget>,
        targets: &dyn WorkloadTargets,
        sampled_at: Instant,
        observed_at: SystemTime,
        recovered: bool,
        fallback_metadata: BTreeMap<String, String>,
    ) -> Result<Option<ResourceSampleDraft>, ProfilingError> {
        let Some(sample) = self
            .sample(trace_id, root, &processes, sampled_at)
            .map_err(|message| ProfilingError::new("resource_profiling_procfs", message))?
        else {
            return Ok(None);
        };
        let total_cpu_percent_millis = sample.cpu_percent_millis;
        let total_rss_kb = sample.rss_kb;
        let total_virtual_memory_kb = sample.virtual_memory_kb;
        let sampled_processes = sample.sampled_processes;
        let root_threads = sample.root_threads;
        let root_comm = sample.root_comm;
        let root_host_pid = targets.coordinates(root)?.map(|host| host.pid);

        let mut metadata = BTreeMap::new();
        if let Some(comm) = root_comm {
            metadata.insert("comm".to_string(), comm);
        }
        metadata.insert(
            "sampled_processes".to_string(),
            sampled_processes.to_string(),
        );
        metadata.insert(
            "children".to_string(),
            sampled_processes.saturating_sub(1).to_string(),
        );
        metadata.insert(
            "threads".to_string(),
            root_threads.unwrap_or_default().to_string(),
        );
        metadata.insert(
            "cpu_baseline_ready".to_string(),
            total_cpu_percent_millis.is_some().to_string(),
        );
        if let Some(cpu) = total_cpu_percent_millis {
            metadata.insert(
                "cpu_percent".to_string(),
                format!("{}.{:03}", cpu / 1000, cpu % 1000),
            );
            metadata.insert("cpu_percent_millis".to_string(), cpu.to_string());
        }
        metadata.insert(
            "cpu_cores".to_string(),
            std::thread::available_parallelism()
                .map(|value| value.get().to_string())
                .map_err(|e| ProfilingError::new("cpu_cores", e.to_string()))?,
        );
        metadata.insert("rss_kb".to_string(), total_rss_kb.to_string());
        metadata.insert(
            "rss_mb".to_string(),
            (total_rss_kb / BYTES_PER_KIB).to_string(),
        );
        metadata.insert(
            "virtual_memory_kb".to_string(),
            total_virtual_memory_kb.to_string(),
        );
        metadata.insert(
            "virtual_memory_mb".to_string(),
            (total_virtual_memory_kb / BYTES_PER_KIB).to_string(),
        );
        metadata.insert(
            "include_children".to_string(),
            self.include_children().to_string(),
        );
        metadata.extend(fallback_metadata);
        Ok(Some(ResourceSampleDraft {
            trace_id,
            observed_at,
            process: root,
            recovered,
            payload: ResourceMetrics {
                scope: if self.include_children() {
                    "process_tree".to_string()
                } else {
                    "process".to_string()
                },
                subject: root_host_pid
                    .map(|pid| format!("pid:{pid}"))
                    .unwrap_or_else(|| root.to_string()),
                accounting_method: ResourceAccountingMethod::ProcfsRssSum,
                accounting_coverage: ResourceAccountingCoverage::Partial,
                sample_kind: ResourceSampleKind::Periodic,
                cpu_percent_millis: total_cpu_percent_millis,
                rss_kb: Some(total_rss_kb),
                virtual_memory_kb: Some(total_virtual_memory_kb),
                process_rss_sum_kb: Some(total_rss_kb),
                metadata,
                ..ResourceMetrics::default()
            },
        }))
    }
}
