use super::cgroup_v2_sampling_backend::*;

impl CgroupV2SamplingBackend {
    /// Shared live/recovered disposition. The selected source remains fixed for the workload lifetime.
    pub(super) fn record_external_outcome(
        &mut self,
        trace_id: TraceId,
        recovered: bool,
        outcome: Result<ExternalSampleOutcome, ProfilingError>,
        drafts: &mut Vec<ResourceSampleDraft>,
        failures: &mut Vec<ResourceSamplingFailure>,
    ) -> bool {
        let (message, consumed) = match outcome {
            Ok(ExternalSampleOutcome::Emitted(draft)) => {
                self.diagnosed_external_failures.remove(&trace_id);
                drafts.push(draft);
                return true;
            }
            Ok(ExternalSampleOutcome::BecameStale) => (
                "external cgroup identity lost; sampling suspended".to_string(),
                true,
            ),
            Ok(ExternalSampleOutcome::Failed(message)) => (message, true),
            Err(error) => (format!("{}: {}", error.code, error.message), true),
        };
        if self.diagnosed_external_failures.insert(trace_id) {
            failures.push(ResourceSamplingFailure {
                trace_id,
                message,
                recovered,
            });
        }
        consumed
    }

    pub(super) fn collect_external_sample(
        &mut self,
        trace_id: TraceId,
        process: ProcessIdentity,
        storage: &mut dyn CgroupV2Persistence,
        targets: &dyn WorkloadTargets,
        sampled_at: Instant,
        observed_at: SystemTime,
        recovered: bool,
    ) -> Result<ExternalSampleOutcome, ProfilingError> {
        let Some(runtime) = self.external.as_mut() else {
            return Ok(ExternalSampleOutcome::Failed(
                "external container cgroup runtime is unavailable".to_string(),
            ));
        };
        let binding = runtime.binding(trace_id).cloned();
        let coordinates = targets.coordinates(process)?;
        let read = runtime.read(
            storage,
            trace_id,
            coordinates.as_ref(),
            crate::SamplingContext {
                sampled_at,
                observed_at,
            },
        )?;
        match read {
            ExternalRead::Counters(sample) => {
                let counters = sample.counters;
                let process_rss = sample.process_rss;
                let cpu_percent_millis = sample.cpu_percent_millis;
                let mut metadata = BTreeMap::new();
                if let Some(binding) = &binding {
                    metadata.insert("ownership".to_string(), "runtime-owned".to_string());
                    metadata.insert("runtime".to_string(), binding.runtime.as_str().to_string());
                    metadata.insert("container_id".to_string(), binding.container_id.to_string());
                    metadata.insert(
                        "relative_path".to_string(),
                        binding.relative_path.display().to_string(),
                    );
                    metadata.insert(
                        "memory_peak_window".to_string(),
                        "cgroup_lifetime".to_string(),
                    );
                    metadata.insert(
                        "attachment_time_unix_seconds".to_string(),
                        binding
                            .created_at
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|duration| duration.as_secs().to_string())
                            .unwrap_or_default(),
                    );
                }
                metadata.insert(
                    "cpu_rate_basis".to_string(),
                    "delta(cpu.stat.usage_usec)/monotonic_elapsed_usec".to_string(),
                );
                if let Some(interval) = sample.cpu_interval_usec {
                    metadata.insert("cpu_rate_interval_usec".to_string(), interval.to_string());
                }
                if let Some(error) = sample.process_rss_error {
                    metadata.insert("process_rss_error".into(), error);
                }
                let mut payload = ResourceMetrics {
                    scope: "container".to_string(),
                    rss_kb: process_rss.map(|rss| rss.rss_sum_kb),
                    process_rss_sum_kb: process_rss.map(|rss| rss.rss_sum_kb),
                    subject: binding
                        .as_ref()
                        .map(|binding| format!("container:{}", binding.container_id))
                        .unwrap_or_else(|| "container".to_string()),
                    accounting_method: ResourceAccountingMethod::CgroupV2,
                    accounting_coverage: ResourceAccountingCoverage::BroaderThanTrace,
                    sample_kind: ResourceSampleKind::Periodic,
                    cpu_percent_millis,
                    metadata,
                    ..ResourceMetrics::default()
                };
                Self::apply_cgroup_counters(&mut payload, counters);
                Ok(ExternalSampleOutcome::Emitted(ResourceSampleDraft {
                    trace_id,
                    observed_at,
                    process,
                    recovered,
                    payload,
                }))
            }
            ExternalRead::BecameStale(reason) => {
                tracing::warn!(
                    %trace_id,
                    reason = reason.as_storage_str(),
                    "external container cgroup binding became stale; sampling suspended"
                );
                Ok(ExternalSampleOutcome::BecameStale)
            }
            ExternalRead::Retryable(message) => Ok(ExternalSampleOutcome::Failed(message)),
        }
    }
}
