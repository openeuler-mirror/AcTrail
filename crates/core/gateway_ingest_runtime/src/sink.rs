use std::fmt;

use sandbox_observation::ObservationBatch;

pub trait SandboxObservationSink: Send + Sync + 'static {
    fn deliver(
        &self,
        gateway_id: u32,
        sb_id: u32,
        batch: ObservationBatch,
    ) -> Result<(), SinkDeliveryError>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SinkDeliveryError {
    stage: String,
    message: String,
}

impl SinkDeliveryError {
    pub fn new(stage: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            stage: stage.into(),
            message: message.into(),
        }
    }

    pub fn stage(&self) -> &str {
        &self.stage
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for SinkDeliveryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.stage, self.message)
    }
}

impl std::error::Error for SinkDeliveryError {}
