use super::cgroup_v2_sampling_backend::*;

impl CgroupV2SamplingBackend {
    pub(super) fn collect_cgroup_sample(
        &mut self,
        trace_id: TraceId,
        process: ProcessIdentity,
        scope: &linux_cgroup::TraceScopePaths,
        sampled_at: Instant,
        observed_at: SystemTime,
        recovered: bool,
    ) -> Result<ResourceSampleDraft, ProfilingError> {
        let sample = self
            .cgroup
            .as_mut()
            .expect("managed scope requires cgroup runtime")
            .sample(
                trace_id,
                process,
                crate::SamplingContext {
                    sampled_at,
                    observed_at,
                },
            )
            .map_err(|error| ProfilingError::new("cgroup_sample", error))?;
        let counters = sample.counters;
        let cpu_percent_millis = sample.cpu_percent_millis;
        let process_rss = sample.process_rss;
        let mut metadata = BTreeMap::from([
            (
                "cgroup_relative_path".to_string(),
                scope.relative_path.display().to_string(),
            ),
            (
                "cpu_rate_basis".to_string(),
                "delta(cpu.stat.usage_usec)/monotonic_elapsed_usec".to_string(),
            ),
        ]);
        if let Some(interval) = sample.cpu_interval_usec {
            metadata.insert("cpu_rate_interval_usec".to_string(), interval.to_string());
        }
        if let Some(error) = sample.process_rss_error {
            metadata.insert("process_rss_error".into(), error);
        }
        if let Some(sample) = process_rss {
            metadata.insert(
                "process_rss_candidate_processes".to_string(),
                sample.candidate_processes.to_string(),
            );
            metadata.insert(
                "process_rss_sampled_processes".to_string(),
                sample.sampled_processes.to_string(),
            );
        }
        for (key, value) in &counters.cpu.unknown {
            metadata.insert(format!("cpu.stat.{key}"), value.clone());
        }
        if let Some(io) = &counters.io {
            for (key, value) in &io.unknown {
                metadata.insert(format!("io.stat.{key}"), value.clone());
            }
        } else {
            metadata.insert(
                "missing_optional_file.io.stat".to_string(),
                "true".to_string(),
            );
        }
        if counters.pids_current.is_none() {
            metadata.insert(
                "missing_optional_file.pids.current".to_string(),
                "true".to_string(),
            );
        }
        let mut payload = ResourceMetrics {
            scope: "trace".to_string(),
            subject: trace_id.to_string(),
            accounting_method: ResourceAccountingMethod::CgroupV2,
            accounting_coverage: ResourceAccountingCoverage::Exact,
            sample_kind: ResourceSampleKind::Periodic,
            cpu_percent_millis,
            rss_kb: process_rss.map(|sample| sample.rss_sum_kb),
            process_rss_sum_kb: process_rss.map(|sample| sample.rss_sum_kb),
            metadata,
            ..ResourceMetrics::default()
        };
        Self::apply_cgroup_counters(&mut payload, counters);
        Ok(ResourceSampleDraft {
            trace_id,
            observed_at,
            process,
            recovered,
            payload,
        })
    }
}
