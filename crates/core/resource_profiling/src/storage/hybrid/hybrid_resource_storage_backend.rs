use super::child_dispatcher::ChildDispatcher;
use crate::{
    model::ResourceSample,
    storage::{ResourceStorageBackend, ResourceStorageError},
};
use std::{sync::Arc, time::Duration};
pub struct HybridResourceStorageBackend {
    pub(super) children: Vec<ChildDispatcher>,
}
impl ResourceStorageBackend for HybridResourceStorageBackend {
    fn write(
        &mut self,
        samples: &[ResourceSample],
        _timeout: Duration,
    ) -> Result<(), ResourceStorageError> {
        let samples: Arc<[ResourceSample]> = samples.into();
        for child in &self.children {
            child.submit(Arc::clone(&samples));
        }
        Ok(())
    }
}
