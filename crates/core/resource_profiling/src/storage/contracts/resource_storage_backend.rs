use super::ResourceStorageError;
use crate::model::ResourceSample;
use std::time::Duration;

pub trait ResourceStorageBackend: Send {
    fn write(
        &mut self,
        samples: &[ResourceSample],
        timeout: Duration,
    ) -> Result<(), ResourceStorageError>;
}
