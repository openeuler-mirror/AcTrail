use super::cgroup_v2_sampling_backend::*;

impl CgroupV2SamplingBackend {
    /// Read-only admission of an attached container's runtime-owned cgroup.
    ///
    /// `require` mode turns an unavailable or unverifiable binding into an
    /// admission failure; otherwise the trace falls back to procfs.
    pub fn admit_container_binding(
        &mut self,
        entry: &WorkloadTarget,
        coordinates: &HostProcessCoordinates,
    ) -> Result<(), ProfilingError> {
        if !entry.requests_resources {
            return Ok(());
        }
        let trace_id = entry.trace_id;
        if !entry.is_container {
            if self.mode == ResourceMetricsMode::Cgroup {
                return Err(ProfilingError::new(
                    "cgroup_admission",
                    "cgroup-v2 requires a managed launch or an external container binding",
                ));
            }
            return Ok(());
        }
        let Some(runtime) = self.bindings.as_mut() else {
            let reason = "external container cgroup accounting is unavailable".to_string();
            if self.mode == ResourceMetricsMode::Cgroup {
                return Err(ProfilingError::new("external_cgroup_admission", reason));
            }
            self.trace_fallback_reasons.insert(trace_id, reason);
            return Ok(());
        };
        match runtime.admit(trace_id, entry.root, coordinates)? {
            CgroupV2BindingAdmission::Bound => {
                self.trace_fallback_reasons.remove(&trace_id);
                Ok(())
            }
            CgroupV2BindingAdmission::Fallback(reason) => {
                if self.mode == ResourceMetricsMode::Cgroup {
                    return Err(ProfilingError::new("cgroup_admission", reason));
                }
                self.trace_fallback_reasons.insert(trace_id, reason);
                Ok(())
            }
        }
    }

    pub fn poll_v2_binding_finalizations(
        &mut self,
        targets: &dyn WorkloadTargets,
        storage: &mut dyn CgroupV2BindingPersistence,
    ) -> Result<Vec<CgroupV2BindingFinalization>, ProfilingError> {
        let Some(runtime) = self.bindings.as_mut() else {
            return Ok(Vec::new());
        };
        let mut terminal_traces = targets
            .traces()
            .into_iter()
            .filter(|trace| trace.terminal)
            .map(|trace| (trace.trace_id, trace.root))
            .collect::<BTreeMap<_, _>>();
        for (trace_id, process) in runtime.recovered_terminal_processes() {
            terminal_traces.entry(trace_id).or_insert(process);
        }
        for (trace_id, process) in runtime.recovered_processes() {
            use crate::model::cgroup_v2_binding::CgroupV2BindingStaleReason;
            if targets.contains_trace(trace_id) {
                continue;
            }
            let binding = match storage.v2_binding(trace_id) {
                Ok(binding) => binding,
                Err(error) => {
                    tracing::warn!(%trace_id,%error,"external recovery binding unavailable; retrying");
                    continue;
                }
            };
            let boot_changed = binding.is_some_and(|binding| {
                binding.stale_reason == Some(CgroupV2BindingStaleReason::HostBootChanged)
            });
            let root_gone = match targets.root_is_gone(process) {
                Ok(gone) => gone,
                Err(error) => {
                    tracing::warn!(%trace_id, message = %error.message, "cannot reconcile recovered container process; retrying");
                    false
                }
            };
            if !boot_changed && !root_gone {
                continue;
            }
            if let Err(error) =
                storage.complete_v2_binding_recovered_trace(trace_id, SystemTime::now())
            {
                tracing::warn!(%trace_id,%error,"external recovered trace completion failed; retrying");
                continue;
            }
            terminal_traces.entry(trace_id).or_insert(process);
        }
        let mut drafts = Vec::new();
        for (trace_id, process) in terminal_traces {
            let context = SamplingContext {
                sampled_at: Instant::now(),
                observed_at: SystemTime::now(),
            };
            let final_draft = match runtime.prepare_final(storage, trace_id, context) {
                Ok(Some(draft)) => draft,
                Ok(None) => continue,
                Err(error) => {
                    tracing::warn!(%trace_id,%error,"external final sample preparation failed; retrying");
                    continue;
                }
            };
            drafts.push(CgroupV2BindingFinalization {
                trace_id,
                observed_at: context.observed_at,
                process,
                payload: Self::build_v2_binding_final_payload(final_draft),
            });
        }
        Ok(drafts)
    }

    pub fn finish_v2_binding_finalization(&mut self, trace_id: TraceId) {
        if let Some(runtime) = self.bindings.as_mut() {
            runtime.remove(trace_id);
        }
        self.diagnosed_binding_failures.remove(&trace_id);
        self.trace_fallback_reasons.remove(&trace_id);
    }

    pub fn v2_binding(
        &self,
        trace_id: TraceId,
    ) -> Option<crate::model::cgroup_v2_binding::CgroupV2Binding> {
        self.bindings.as_ref()?.binding(trace_id).cloned()
    }

    pub fn prune_forgotten(&mut self, targets: &dyn WorkloadTargets) {
        self.trace_fallback_reasons
            .retain(|id, _| targets.contains_trace(*id));
    }

    pub fn v2_binding_barrier_ready(&self, trace_id: TraceId) -> bool {
        self.bindings
            .as_ref()
            .is_none_or(|runtime| !runtime.has_pending(trace_id))
    }
}
