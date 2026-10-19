use model_core::ids::TraceId;
use resource_profiling::model::resource_scope::ResourceScopeLifecycleState;
use resource_profiling::model::resource_scope::TraceResourceScope;
use resource_profiling::{ManagedRecoveryRecord, ManagedScopePersistence, ProfilingError};
use storage_core::StorageBackend;

pub(super) struct ManagedScopeStorage<'a> {
    pub storage: &'a mut dyn StorageBackend,
}
impl ManagedScopeStorage<'_> {
    fn complete_persisted_finalization_barriers(
        &mut self,
        registered: &[TraceResourceScope],
    ) -> Result<(), ProfilingError> {
        for scope in registered
            .iter()
            .filter(|scope| !scope.lifecycle_state.is_live())
        {
            let Some(mut trace) = self
                .storage
                .get_trace(scope.trace_id)
                .map_err(|error| ProfilingError::new(error.stage, error.message))?
            else {
                return Err(ProfilingError::new(
                    "cgroup_recovery",
                    format!(
                        "finalized resource scope {} has no trace record",
                        scope.trace_id
                    ),
                ));
            };
            if trace.lifecycle_state.is_terminal() {
                continue;
            }
            trace.lifecycle_state = model_core::trace::TraceLifecycleState::Completed;
            trace.timings.completed_at = Some(scope.updated_at);
            if scope.lifecycle_state == ResourceScopeLifecycleState::Orphaned {
                trace.health = model_core::trace::TraceHealth::Degraded;
            }
            self.storage
                .create_trace(trace)
                .map_err(|error| ProfilingError::new(error.stage, error.message))?;
        }
        Ok(())
    }
}
impl ManagedScopePersistence for ManagedScopeStorage<'_> {
    fn prepare_recovery(&mut self) -> Result<Vec<ManagedRecoveryRecord>, ProfilingError> {
        let registered = self
            .storage
            .list_resource_scopes()
            .map_err(|e| ProfilingError::new(e.stage, e.message))?;
        self.complete_persisted_finalization_barriers(&registered)?;
        self.storage
            .list_resource_scopes()
            .map_err(|e| ProfilingError::new(e.stage, e.message))?
            .into_iter()
            .map(|scope| {
                let trace = self
                    .storage
                    .get_trace(scope.trace_id)
                    .map_err(|e| ProfilingError::new(e.stage, e.message))?
                    .ok_or_else(|| {
                        ProfilingError::new(
                            "cgroup_recovery",
                            format!("resource scope {} has no trace", scope.trace_id),
                        )
                    })?;
                Ok(ManagedRecoveryRecord {
                    scope,
                    root: trace.root_process_identity,
                })
            })
            .collect()
    }
    fn finalize_lost_scope(
        &mut self,
        trace_id: TraceId,
        reason: &str,
    ) -> Result<(), ProfilingError> {
        use model_core::diagnostics::{DiagnosticKind, DiagnosticRecord, DiagnosticSeverity};
        use model_core::ids::DiagnosticId;
        use model_core::trace::{TraceHealth, TraceLifecycleState};
        let error =
            |error: storage_core::StorageError| ProfilingError::new(error.stage, error.message);
        let scope = self
            .storage
            .get_resource_scope(trace_id)
            .map_err(error)?
            .ok_or_else(|| {
                ProfilingError::new("cgroup_recovery", "resource scope record is missing")
            })?;
        if !scope.lifecycle_state.is_live() {
            return Ok(());
        }
        let mut trace = self
            .storage
            .get_trace(trace_id)
            .map_err(error)?
            .ok_or_else(|| {
                ProfilingError::new("cgroup_recovery", "resource scope has no trace record")
            })?;
        let now = std::time::SystemTime::now();
        let diagnostic_id =
            DiagnosticId::new(self.storage.next_diagnostic_id_seed().map_err(error)?);
        if !trace.lifecycle_state.is_terminal() {
            trace.lifecycle_state = TraceLifecycleState::Completed;
            trace.timings.completed_at = Some(now);
        }
        trace.health = TraceHealth::Degraded;
        let diagnostic = DiagnosticRecord::new(
            diagnostic_id,
            Some(trace_id),
            DiagnosticKind::RuntimeFailure,
            DiagnosticSeverity::Warning,
            now,
            reason,
        )
        .with_metadata("resource_scope_recovery", "lost");
        let transaction = self.storage.begin().map_err(error)?;
        let result = (|| {
            self.storage.update_resource_scope_state(
                trace_id,
                ResourceScopeLifecycleState::Orphaned,
                now,
            )?;
            self.storage.create_trace(trace)?;
            self.storage.append_diagnostic(diagnostic)
        })();
        match result {
            Ok(()) => transaction.commit().map_err(error),
            Err(failure) => {
                transaction.rollback().map_err(error)?;
                Err(error(failure))
            }
        }
    }
}
