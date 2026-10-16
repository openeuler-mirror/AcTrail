use super::{ResourceStorageBackend, ResourceStorageError};
use crate::model::ResourceSample;
use std::{
    sync::{
        Arc,
        mpsc::{SyncSender, sync_channel},
    },
    time::Duration,
};
pub(super) struct QueuedResourceStorageBackend {
    sender: SyncSender<Arc<[ResourceSample]>>,
}
impl QueuedResourceStorageBackend {
    pub(super) fn new(
        mut backend: Box<dyn ResourceStorageBackend>,
        capacity: usize,
    ) -> Result<Self, ResourceStorageError> {
        let (sender, receiver) = sync_channel::<Arc<[ResourceSample]>>(capacity);
        std::thread::Builder::new()
            .name("resource-storage".into())
            .spawn(move || {
                for batch in receiver {
                    if let Err(error) = backend.write(&batch, Duration::MAX) {
                        tracing::warn!(%error,"resource storage write failed");
                    }
                }
            })
            .map_err(|e| ResourceStorageError(e.to_string()))?;
        Ok(Self { sender })
    }
}
impl ResourceStorageBackend for QueuedResourceStorageBackend {
    fn write(
        &mut self,
        samples: &[ResourceSample],
        _timeout: Duration,
    ) -> Result<(), ResourceStorageError> {
        self.sender.try_send(samples.into()).map_err(|_| {
            ResourceStorageError(
                "resource storage queue full or worker unavailable; batch dropped".into(),
            )
        })
    }
}
