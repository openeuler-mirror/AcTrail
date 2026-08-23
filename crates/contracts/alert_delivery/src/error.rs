use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeliveryCodecError {
    stage: &'static str,
    message: String,
}

impl DeliveryCodecError {
    pub(crate) fn new(stage: &'static str, message: impl Into<String>) -> Self {
        Self {
            stage,
            message: message.into(),
        }
    }

    pub fn stage(&self) -> &'static str {
        self.stage
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for DeliveryCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.stage, self.message)
    }
}

impl std::error::Error for DeliveryCodecError {}
