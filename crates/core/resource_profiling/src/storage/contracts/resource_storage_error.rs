use std::fmt;
#[derive(Debug)]
pub struct ResourceStorageError(pub String);
impl fmt::Display for ResourceStorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for ResourceStorageError {}
impl From<serde_json::Error> for ResourceStorageError {
    fn from(error: serde_json::Error) -> Self {
        Self(error.to_string())
    }
}
