use super::cgroup_v2_sampling_backend::*;

impl CgroupV2SamplingBackend {
    /// Admit a daemon-controlled, SIGSTOPped launch into its trace scope.
    ///
    /// Scope creation/controller setup may fall back in `auto` mode. Moving the
    /// PID is the commit boundary: any error at or after that point is fatal.
    pub fn admit_stopped_launch(
        &mut self,
        entry: &WorkloadTarget,
        pid: u32,
        launch_mode: bool,
    ) -> Result<Option<TraceResourceScope>, ProfilingError> {
        if !launch_mode || !entry.requests_resources {
            return Ok(None);
        }
        let trace_id = entry.trace_id;
        if entry.is_container {
            if self.mode == ResourceMetricsMode::Cgroup {
                return Err(ProfilingError::new(
                    "cgroup_admission",
                    "controlled container launch has no managed host scope",
                ));
            }
            self.trace_fallback_reasons.insert(
                trace_id,
                "controlled container/VM launch is outside the host-managed cgroup scope"
                    .to_string(),
            );
            return Ok(None);
        }
        let Some(runtime) = self.cgroup.as_mut() else {
            if self.mode == ResourceMetricsMode::Cgroup {
                return Err(ProfilingError::new(
                    "cgroup_admission",
                    self.cgroup_fallback_reason
                        .clone()
                        .unwrap_or_else(|| "managed cgroup admission unavailable".into()),
                ));
            }
            return Ok(None);
        };
        match runtime.admit_stopped_launch(trace_id, pid, self.mode)? {
            CgroupAdmission::Managed(scope) => Ok(Some(scope)),
            CgroupAdmission::Fallback(reason) => {
                tracing::warn!(%trace_id, %reason, "cgroup launch admission fell back to procfs");
                self.trace_fallback_reasons.insert(trace_id, reason);
                Ok(None)
            }
        }
    }

    pub fn activate_scope(&mut self, scope: &TraceResourceScope) -> Result<(), ProfilingError> {
        let runtime = self.cgroup.as_mut().ok_or_else(|| {
            ProfilingError::new(
                "cgroup_scope_activation",
                "cannot activate a scope without a cgroup runtime",
            )
        })?;
        runtime.activate_scope(scope)?;
        self.trace_fallback_reasons.remove(&scope.trace_id);
        Ok(())
    }

    /// Read-only admission of an attached container's runtime-owned cgroup.
    ///
    /// `require` mode turns an unavailable or unverifiable binding into an
    /// admission failure; otherwise the trace falls back to procfs.
    pub fn admit_external_container(
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
        let Some(runtime) = self.external.as_mut() else {
            let reason = "external container cgroup accounting is unavailable".to_string();
            if self.mode == ResourceMetricsMode::Cgroup {
                return Err(ProfilingError::new("external_cgroup_admission", reason));
            }
            self.trace_fallback_reasons.insert(trace_id, reason);
            return Ok(());
        };
        match runtime.admit(trace_id, entry.root, coordinates)? {
            ExternalAdmission::Bound => {
                self.trace_fallback_reasons.remove(&trace_id);
                Ok(())
            }
            ExternalAdmission::Fallback(reason) => {
                if self.mode == ResourceMetricsMode::Cgroup {
                    return Err(ProfilingError::new("cgroup_admission", reason));
                }
                self.trace_fallback_reasons.insert(trace_id, reason);
                Ok(())
            }
        }
    }

    pub fn poll_finalizations(
        &mut self,
        targets: &dyn WorkloadTargets,
    ) -> Result<ResourceFinalizationPoll, ProfilingError> {
        let Some(runtime) = self.cgroup.as_ref() else {
            return Ok(ResourceFinalizationPoll::default());
        };
        let mut candidates = targets
            .traces()
            .into_iter()
            .filter(|trace| trace.draining || trace.terminal)
            .filter_map(|trace| {
                runtime
                    .scope(trace.trace_id)
                    .cloned()
                    .map(|scope| (trace.trace_id, trace.root, scope, false))
            })
            .collect::<Vec<_>>();
        for (trace_id, process) in runtime.recovered_processes() {
            if candidates
                .iter()
                .any(|(candidate, _, _, _)| *candidate == trace_id)
            {
                continue;
            }
            let Some(scope) = runtime.scope(trace_id).cloned() else {
                continue;
            };
            let already_waiting = self.finalization_started_at.contains_key(&trace_id);
            let empty = runtime
                .populated(&scope)
                .map(|populated| !populated)
                .unwrap_or(false);
            let root_gone = match targets.root_is_gone(process) {
                Ok(gone) => gone,
                Err(error) => {
                    tracing::warn!(%trace_id,%error,"managed root liveness unavailable; retrying");
                    false
                }
            };
            if already_waiting || empty || root_gone {
                candidates.push((trace_id, process, scope, true));
            }
        }
        let now = Instant::now();
        let observed_at = SystemTime::now();
        let timeout = Duration::from_millis(self.finalization_timeout_ms);
        let mut poll = ResourceFinalizationPoll::default();

        for (trace_id, process, scope, recovered) in candidates {
            let started_at = *self.finalization_started_at.entry(trace_id).or_insert(now);
            poll.waiting.push(trace_id);
            let timed_out = now.saturating_duration_since(started_at) >= timeout;
            let populated = self
                .cgroup
                .as_ref()
                .expect("candidate requires cgroup runtime")
                .populated(&scope);
            let mut finalization_timed_out = match populated {
                Ok(true) if !timed_out => continue,
                Ok(false) => false,
                Ok(true) => true,
                Err(error) if !timed_out => {
                    tracing::warn!(%trace_id, %error, "cgroup finalization readiness read failed");
                    continue;
                }
                Err(error) => {
                    tracing::warn!(
                        %trace_id,
                        %error,
                        "cgroup finalization readiness read failed at timeout"
                    );
                    true
                }
            };

            let mut sample = match self.collect_cgroup_sample(
                trace_id,
                process,
                &scope,
                now,
                observed_at,
                recovered,
            ) {
                Ok(sample) => sample,
                Err(error) if timed_out => {
                    finalization_timed_out = true;
                    ResourceSampleDraft {
                        trace_id,
                        observed_at,
                        process,
                        recovered,
                        payload: ResourceMetrics {
                            scope: "trace".to_string(),
                            subject: trace_id.to_string(),
                            accounting_method: ResourceAccountingMethod::CgroupV2,
                            accounting_coverage: ResourceAccountingCoverage::Partial,
                            sample_kind: ResourceSampleKind::Final,
                            metadata: BTreeMap::from([
                                ("alert".to_string(), "false".to_string()),
                                ("finalization_timeout".to_string(), "true".to_string()),
                                ("finalization_read_error".to_string(), error.message),
                            ]),
                            ..ResourceMetrics::default()
                        },
                    }
                }
                Err(error) => {
                    tracing::warn!(%trace_id, error = %error.message,
                        "cgroup final counters unavailable; retrying until deadline");
                    continue;
                }
            };
            sample.payload.sample_kind = ResourceSampleKind::Final;
            if finalization_timed_out {
                sample.payload.accounting_coverage = ResourceAccountingCoverage::Partial;
                sample
                    .payload
                    .metadata
                    .insert("finalization_timeout".to_string(), "true".to_string());
                sample.payload.metadata.insert(
                    "finalization_timeout_ms".to_string(),
                    self.finalization_timeout_ms.to_string(),
                );
            }
            poll.ready.push(ResourceFinalizationDraft {
                trace_id,
                sample,
                timed_out: finalization_timed_out,
                recovered,
            });
        }
        Ok(poll)
    }

    pub fn finish_finalization(&mut self, trace_id: TraceId, remove_scope: bool) {
        self.finalized_barriers.insert(trace_id);
        self.diagnosed_cgroup_failures.remove(&trace_id);
        self.finalization_started_at.remove(&trace_id);
        if let Some(runtime) = self.cgroup.as_mut() {
            runtime.finish(trace_id, remove_scope);
        }
    }

    pub fn poll_external_finalizations(
        &mut self,
        targets: &dyn WorkloadTargets,
        storage: &mut dyn CgroupV2Persistence,
    ) -> Result<Vec<ExternalFinalizationDraft>, ProfilingError> {
        let Some(runtime) = self.external.as_mut() else {
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
            use crate::model::external_cgroup::ExternalBindingStaleReason;
            if targets.contains_trace(trace_id) {
                continue;
            }
            let binding = match storage.binding(trace_id) {
                Ok(binding) => binding,
                Err(error) => {
                    tracing::warn!(%trace_id,%error,"external recovery binding unavailable; retrying");
                    continue;
                }
            };
            let boot_changed = binding.is_some_and(|binding| {
                binding.stale_reason == Some(ExternalBindingStaleReason::HostBootChanged)
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
            if let Err(error) = storage.complete_recovered_trace(trace_id, SystemTime::now()) {
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
            drafts.push(ExternalFinalizationDraft {
                trace_id,
                observed_at: context.observed_at,
                process,
                payload: Self::build_external_final_payload(final_draft),
            });
        }
        Ok(drafts)
    }

    pub fn finish_external_finalization(&mut self, trace_id: TraceId) {
        if let Some(runtime) = self.external.as_mut() {
            runtime.remove(trace_id);
        }
        self.diagnosed_external_failures.remove(&trace_id);
        self.trace_fallback_reasons.remove(&trace_id);
    }

    pub fn external_binding(
        &self,
        trace_id: TraceId,
    ) -> Option<crate::model::external_cgroup::ExternalCgroupBinding> {
        self.external.as_ref()?.binding(trace_id).cloned()
    }

    pub fn prune_forgotten(&mut self, targets: &dyn WorkloadTargets) {
        self.finalized_barriers
            .retain(|id| targets.contains_trace(*id));
        self.trace_fallback_reasons
            .retain(|id, _| targets.contains_trace(*id));
    }

    pub fn resource_barrier_ready(&self, trace_id: TraceId) -> bool {
        self.cgroup
            .as_ref()
            .is_none_or(|runtime| !runtime.has_scope(trace_id))
    }

    pub fn completed_managed_barrier(&self, trace_id: TraceId) -> bool {
        self.finalized_barriers.contains(&trace_id)
    }

    pub fn has_pending_barrier(&self, trace_id: TraceId) -> bool {
        self.cgroup
            .as_ref()
            .is_some_and(|runtime| runtime.has_scope(trace_id))
    }

    pub fn external_barrier_ready(&self, trace_id: TraceId) -> bool {
        self.external
            .as_ref()
            .is_none_or(|runtime| !runtime.has_pending(trace_id))
    }

    pub fn force_pending_finalizations_due(&mut self) {
        let elapsed = Duration::from_millis(self.finalization_timeout_ms);
        let due = Instant::now()
            .checked_sub(elapsed)
            .unwrap_or_else(Instant::now);
        for started_at in self.finalization_started_at.values_mut() {
            *started_at = due;
        }
    }
}
