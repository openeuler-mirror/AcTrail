//! Maps local and remote failures to the shared checkpoint error types.

use checkpoint::{BackendError, BackendErrorKind};

#[derive(Debug)]
pub(super) enum Error {
    Configuration(String),
    Snapshot(String),
    Transport(std::io::Error),
    Operation(String),
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::Transport(error)
    }
}

impl From<Error> for BackendError {
    fn from(error: Error) -> Self {
        let (kind, message) = match error {
            Error::Configuration(message) => (BackendErrorKind::InvalidConfiguration, message),
            Error::Snapshot(message) => (BackendErrorKind::InvalidSnapshot, message),
            Error::Transport(error) => (BackendErrorKind::Unavailable, error.to_string()),
            Error::Operation(message) => (BackendErrorKind::OperationFailed, message),
        };
        Self {
            kind,
            message: message.into(),
        }
    }
}
