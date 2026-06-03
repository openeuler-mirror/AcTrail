//! Error type for payload processing.

use std::error::Error;
use std::fmt::{Display, Formatter};

pub type CoreResult<T> = Result<T, CoreError>;

#[derive(Debug)]
pub struct CoreError {
    message: String,
}

impl CoreError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl Display for CoreError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for CoreError {}
