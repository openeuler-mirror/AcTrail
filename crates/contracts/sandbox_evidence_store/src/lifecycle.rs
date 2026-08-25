use std::time::Duration;

use crate::SandboxEvidenceShutdownError;

pub trait SandboxEvidenceLifecyclePort: Send {
    fn shutdown(&mut self, timeout: Duration) -> Result<(), SandboxEvidenceShutdownError>;
}
