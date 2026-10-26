use crate::{
    model::ResourceSample,
    storage::{ResourceStorageBackend, ResourceStorageError},
};
use std::{
    sync::{
        Arc,
        mpsc::{SyncSender, TrySendError, sync_channel},
    },
    time::Duration,
};
pub(super) struct ChildDispatcher {
    name: String,
    sender: SyncSender<Arc<[ResourceSample]>>,
}
impl ChildDispatcher {
    pub(super) fn new(
        name: String,
        mut backend: Box<dyn ResourceStorageBackend>,
        capacity: usize,
        timeout: Duration,
    ) -> Result<Self, ResourceStorageError> {
        let (sender, receiver) = sync_channel::<Arc<[ResourceSample]>>(capacity);
        let thread_name = name.clone();
        std::thread::Builder::new()
            .name(format!("resource-{name}"))
            .spawn(move || {
                for samples in receiver {
                    if let Err(error) = backend.write(&samples, timeout) {
                        tracing::warn!(target_backend=%thread_name,%error,"resource output failed");
                    }
                }
            })
            .map_err(|e| ResourceStorageError(format!("resource writer thread: {e}")))?;
        Ok(Self { name, sender })
    }
    pub(super) fn submit(&self, samples: Arc<[ResourceSample]>) {
        match self.sender.try_send(samples) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                tracing::warn!(target_backend=%self.name,"resource output queue full; batch dropped")
            }
            Err(TrySendError::Disconnected(_)) => {
                tracing::warn!(target_backend=%self.name,"resource output worker unavailable; batch dropped")
            }
        }
    }
}
