//! Storage facade error type.

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StorageError {
    pub stage: String,
    pub message: String,
    pub supplement_rejection: Option<semantic_action::SemanticAttributeSupplementRejection>,
}

impl StorageError {
    pub fn new(stage: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            stage: stage.into(),
            message: message.into(),
            supplement_rejection: None,
        }
    }
}

impl From<semantic_action::SemanticActionStoreError> for StorageError {
    fn from(error: semantic_action::SemanticActionStoreError) -> Self {
        Self {
            stage: error.stage,
            message: error.message,
            supplement_rejection: error.supplement_rejection,
        }
    }
}

impl From<store_read_contract::ReadError> for StorageError {
    fn from(error: store_read_contract::ReadError) -> Self {
        Self::new(error.stage, error.message)
    }
}

impl From<store_retention_contract::RetentionError> for StorageError {
    fn from(error: store_retention_contract::RetentionError) -> Self {
        Self::new(error.stage, error.message)
    }
}

impl From<store_snapshot_contract::SnapshotError> for StorageError {
    fn from(error: store_snapshot_contract::SnapshotError) -> Self {
        Self::new(error.stage, error.message)
    }
}

impl From<store_tx_contract::StorageTransactionError> for StorageError {
    fn from(error: store_tx_contract::StorageTransactionError) -> Self {
        Self::new(error.stage, error.message)
    }
}

impl From<store_write_contract::WriteError> for StorageError {
    fn from(error: store_write_contract::WriteError) -> Self {
        Self::new(error.stage, error.message)
    }
}
