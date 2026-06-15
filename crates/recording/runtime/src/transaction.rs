use storage_core::{StorageBackend, StorageTransaction};

use crate::RecordingError;

pub struct RecordingTransaction {
    transaction: Box<dyn StorageTransaction>,
}

impl RecordingTransaction {
    pub fn begin(storage: &mut dyn StorageBackend) -> Result<Self, RecordingError> {
        storage
            .begin()
            .map(|transaction| Self { transaction })
            .map_err(RecordingError::from)
    }

    pub fn commit_or_rollback<T, E>(
        self,
        write_result: Result<T, E>,
        map_commit_error: impl FnOnce(RecordingError) -> E,
    ) -> Result<T, E> {
        match write_result {
            Ok(value) => self
                .transaction
                .commit()
                .map(|()| value)
                .map_err(|error| map_commit_error(RecordingError::from(error))),
            Err(error) => {
                let _ = self.transaction.rollback();
                Err(error)
            }
        }
    }

    pub fn commit_or_rollback_then<T, R, E>(
        self,
        write_result: Result<T, E>,
        map_commit_error: impl FnOnce(RecordingError) -> E,
        post_commit: impl FnOnce(T) -> Result<R, E>,
    ) -> Result<R, E> {
        let value = self.commit_or_rollback(write_result, map_commit_error)?;
        // Post-commit work is intentionally outside rollback scope.
        post_commit(value)
    }
}
