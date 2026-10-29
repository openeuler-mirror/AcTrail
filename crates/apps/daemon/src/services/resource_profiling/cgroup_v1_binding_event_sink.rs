use crate::services::attach::StorageAttachService;
use control_contract::reply::ControlError;
use model_core::ids::TraceId;
use std::time::SystemTime;
use trace_runtime::TraceRuntime;
impl StorageAttachService {
    pub(in crate::services) fn drain_v1_binding_finalizations(&mut self, traces: &TraceRuntime) {
        let drafts = self.resource_metrics.poll_v1_binding_finalizations(
            traces,
            &self.process_registry,
            self.storage.as_mut(),
        );
        for draft in drafts {
            let id = draft;
            if let Err(error) = self.persist_v1_binding_finalization(draft) {
                tracing::warn!(%id,message=%error.message,"v1 final persistence failed; retrying");
            }
        }
    }

    fn persist_v1_binding_finalization(&mut self, id: TraceId) -> Result<(), ControlError> {
        self.storage
            .close_cgroup_v1_binding(id, SystemTime::now())
            .map_err(|e| ControlError::new(e.stage, e.message))?;
        self.resource_metrics.finish_v1_binding_finalization(id);
        Ok(())
    }
}
