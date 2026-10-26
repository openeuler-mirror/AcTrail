use super::cgroup_v2_sampling_backend::*;
use crate::WorkloadSamplingBackend;
impl<'a> WorkloadSamplingBackend<dyn CgroupV2Persistence + 'a> for CgroupV2SamplingBackend {
    fn maintain(&mut self) {
        if let Some(runtime) = self.cgroup.as_mut() {
            runtime.cleanup_abandoned_admissions();
        }
    }
    fn collect(
        &mut self,
        targets: &dyn WorkloadTargets,
        storage: &mut (dyn CgroupV2Persistence + 'a),
        context: SamplingContext,
        selection: &mut crate::SamplingSelection,
    ) -> Result<ResourceMetricsDrain, ProfilingError> {
        let sampled_at = context.sampled_at;
        let observed_at = context.observed_at;
        let mut drafts = Vec::new();
        let mut failures = Vec::new();
        for trace in targets.traces() {
            if trace.terminal
                && self.resource_barrier_ready(trace.trace_id)
                && self.external_barrier_ready(trace.trace_id)
            {
                selection.claim(trace.trace_id);
                continue;
            }
            if selection.is_claimed(trace.trace_id) {
                continue;
            }
            if self.finalized_barriers.contains(&trace.trace_id) {
                selection.claim(trace.trace_id);
                continue;
            }
            let entry = &trace;
            if !entry.requests_resources {
                continue;
            }
            let cgroup_scope = self
                .cgroup
                .as_ref()
                .and_then(|runtime| runtime.scope(trace.trace_id))
                .cloned();
            if let Some(scope) = cgroup_scope {
                selection.claim(trace.trace_id);
                match self.collect_cgroup_sample(
                    trace.trace_id,
                    entry.root,
                    &scope,
                    sampled_at,
                    observed_at,
                    false,
                ) {
                    Ok(draft) => {
                        self.diagnosed_cgroup_failures.remove(&trace.trace_id);
                        drafts.push(draft);
                    }
                    Err(error) => {
                        if self.diagnosed_cgroup_failures.insert(trace.trace_id) {
                            failures.push(ResourceSamplingFailure {
                                trace_id: trace.trace_id,
                                message: format!("{}: {}", error.code, error.message),
                                recovered: false,
                            });
                        }
                    }
                }
                continue;
            }
            if self
                .external
                .as_ref()
                .is_some_and(|runtime| runtime.is_active(trace.trace_id))
            {
                selection.claim(trace.trace_id);
                let outcome = self.collect_external_sample(
                    trace.trace_id,
                    entry.root,
                    storage,
                    targets,
                    sampled_at,
                    observed_at,
                    false,
                );
                if self.record_external_outcome(
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
                .external
                .as_ref()
                .is_some_and(|runtime| runtime.is_stale(trace.trace_id))
            {
                selection.claim(trace.trace_id);
                continue;
            }
            selection.fallback_metadata(trace.trace_id).insert(
                "cgroup_fallback_reason".into(),
                self.trace_fallback_reasons
                    .get(&trace.trace_id)
                    .cloned()
                    .or_else(|| self.cgroup_fallback_reason.clone())
                    .unwrap_or_else(|| "trace_has_no_managed_cgroup_scope".into()),
            );
        }
        let recovered_scopes = self
            .cgroup
            .as_ref()
            .map(|runtime| {
                runtime
                    .recovered_processes()
                    .filter_map(|(trace_id, process)| {
                        if targets.contains_trace(trace_id) {
                            return None;
                        }
                        runtime
                            .scope(trace_id)
                            .cloned()
                            .map(|scope| (trace_id, process, scope))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for (trace_id, process, scope) in recovered_scopes {
            if selection.is_claimed(trace_id) {
                continue;
            }
            selection.claim(trace_id);
            match self.collect_cgroup_sample(
                trace_id,
                process,
                &scope,
                sampled_at,
                observed_at,
                true,
            ) {
                Ok(draft) => {
                    self.diagnosed_cgroup_failures.remove(&trace_id);
                    drafts.push(draft);
                }
                Err(error) => {
                    if self.diagnosed_cgroup_failures.insert(trace_id) {
                        failures.push(ResourceSamplingFailure {
                            trace_id,
                            message: format!("{}: {}", error.code, error.message),
                            recovered: true,
                        });
                    }
                }
            }
        }
        let recovered_external = self
            .external
            .as_ref()
            .map(|runtime| {
                runtime
                    .recovered_processes()
                    .filter(|(trace_id, _)| !targets.contains_trace(*trace_id))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for (trace_id, process) in recovered_external {
            if selection.is_claimed(trace_id) {
                continue;
            }
            selection.claim(trace_id);
            let external_active = self
                .external
                .as_ref()
                .is_some_and(|runtime| runtime.is_active(trace_id));
            if external_active {
                let outcome = self.collect_external_sample(
                    trace_id,
                    process,
                    storage,
                    targets,
                    sampled_at,
                    observed_at,
                    true,
                );
                if self.record_external_outcome(trace_id, true, outcome, &mut drafts, &mut failures)
                {
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
