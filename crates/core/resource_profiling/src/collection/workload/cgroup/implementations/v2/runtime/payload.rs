use super::cgroup_v2_sampling_backend::*;
impl CgroupV2SamplingBackend {
    pub(super) fn build_v2_binding_final_payload(
        draft: CgroupV2BindingFinalRead,
    ) -> ResourceMetrics {
        let binding = &draft.binding;
        let mut metadata = BTreeMap::new();
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
        if let Some(seconds) = Self::unix_seconds(binding.created_at) {
            metadata.insert(
                "attachment_time_unix_seconds".to_string(),
                seconds.to_string(),
            );
        }
        if let Some(reason) = binding.stale_reason {
            metadata.insert(
                "fallback_from".to_string(),
                "external_container_cgroup".to_string(),
            );
            metadata.insert(
                "stale_reason".to_string(),
                reason.as_storage_str().to_string(),
            );
        }
        if let Some(last_good_at) = binding.last_good_at {
            if let Some(seconds) = Self::unix_seconds(last_good_at) {
                metadata.insert("last_good_at_unix_seconds".to_string(), seconds.to_string());
            }
        }
        if let Some(error) = &draft.read_error {
            metadata.insert("finalization_read_error".to_string(), error.clone());
        }
        let has_counters = draft.sample.is_some();
        let mut payload = ResourceMetrics {
            scope: "container".to_string(),
            subject: format!("container:{}", binding.container_id),
            accounting_method: ResourceAccountingMethod::CgroupV2,
            accounting_coverage: if has_counters {
                ResourceAccountingCoverage::BroaderThanTrace
            } else {
                ResourceAccountingCoverage::Partial
            },
            sample_kind: ResourceSampleKind::Final,
            metadata,
            ..ResourceMetrics::default()
        };
        if let Some(sample) = draft.sample {
            payload.cpu_percent_millis = sample.cpu_percent_millis;
            payload.process_rss_sum_kb = sample.process_rss.map(|rss| rss.rss_sum_kb);
            payload.rss_kb = payload.process_rss_sum_kb;
            if let Some(error) = sample.process_rss_error {
                payload.metadata.insert("process_rss_error".into(), error);
            }
            apply_cgroup_counters(&mut payload, sample.counters);
        }
        payload
    }

    pub(super) fn unix_seconds(time: SystemTime) -> Option<u64> {
        time.duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .ok()
    }
}
