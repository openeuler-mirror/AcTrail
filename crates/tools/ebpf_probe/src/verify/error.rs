//! Stage-aware failures for live verification.

use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LiveVerificationStage {
    Setup,
    LoadAttach,
    WorkloadEventDrain,
    RetainedObservation,
}

impl LiveVerificationStage {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Setup => "setup",
            Self::LoadAttach => "load_attach",
            Self::WorkloadEventDrain => "workload_event_drain",
            Self::RetainedObservation => "retained_observation",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LiveVerificationError {
    pub stage: LiveVerificationStage,
    pub message: String,
}

impl LiveVerificationError {
    pub fn new(stage: LiveVerificationStage, message: impl Into<String>) -> Self {
        Self {
            stage,
            message: message.into(),
        }
    }
}

impl fmt::Display for LiveVerificationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "verification_stage={} status=failed\nverification_error={}",
            self.stage.as_str(),
            self.message
        )
    }
}
