use super::cgroup_v2_sampling_backend::*;
use crate::WorkloadSamplingBackend;
/// Samples attached containers' runtime-owned cgroups. Terminal-trace barriers
/// and managed scopes are claimed earlier by `CgroupSamplingBackend`.
impl<'a> WorkloadSamplingBackend<dyn CgroupV2BindingPersistence + 'a> for CgroupV2SamplingBackend {
    fn maintain(&mut self) {}
    fn collect(
        &mut self,
        targets: &dyn WorkloadTargets,
        storage: &mut (dyn CgroupV2BindingPersistence + 'a),
        context: SamplingContext,
        selection: &mut crate::SamplingSelection,
    ) -> Result<ResourceMetricsDrain, ProfilingError> {
        let sampled_at = context.sampled_at;
        let observed_at = context.observed_at;
        let mut drafts = Vec::new();
        let mut failures = Vec::new();
        for trace in targets.traces() {
            if selection.is_claimed(trace.trace_id) || !trace.requests_resources {
                continue;
            }
            if self
                .bindings
                .as_ref()
                .is_some_and(|runtime| runtime.is_active(trace.trace_id))
            {
                selection.claim(trace.trace_id);
                let outcome = self.collect_v2_binding_sample(
                    trace.trace_id,
                    trace.root,
                    storage,
                    targets,
                    sampled_at,
                    observed_at,
                    false,
                );
                if self.record_v2_binding_outcome(
                    trace.trace_id,
                    false,
                    outcome,
                    &mut drafts,
                    &mut failures,
                ) {
                    continue;
                }
            }
            if self
                .bindings
                .as_ref()
                .is_some_and(|runtime| runtime.is_stale(trace.trace_id))
            {
                selection.claim(trace.trace_id);
            }
        }
        let recovered_bindings = self
            .bindings
            .as_ref()
            .map(|runtime| {
                runtime
                    .recovered_processes()
                    .filter(|(trace_id, _)| !targets.contains_trace(*trace_id))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for (trace_id, process) in recovered_bindings {
            if selection.is_claimed(trace_id) {
                continue;
            }
            selection.claim(trace_id);
            let binding_active = self
                .bindings
                .as_ref()
                .is_some_and(|runtime| runtime.is_active(trace_id));
            if binding_active {
                let outcome = self.collect_v2_binding_sample(
                    trace_id,
                    process,
                    storage,
                    targets,
                    sampled_at,
                    observed_at,
                    true,
                );
                if self.record_v2_binding_outcome(
                    trace_id,
                    true,
                    outcome,
                    &mut drafts,
                    &mut failures,
                ) {
                    continue;
                }
            }
        }

        Ok(ResourceMetricsDrain {
            samples: drafts,
            failures,
        })
    }
}
