use export_core::ExportError;
use storage_core::StorageError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordingError {
    pub stage: String,
    pub message: String,
}

impl RecordingError {
    pub fn new(stage: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            stage: stage.into(),
            message: message.into(),
        }
    }
}

impl From<StorageError> for RecordingError {
    fn from(error: StorageError) -> Self {
        Self::new(error.stage, error.message)
    }
}

impl From<ExportError> for RecordingError {
    fn from(error: ExportError) -> Self {
        Self::new(error.code, error.message)
    }
}
