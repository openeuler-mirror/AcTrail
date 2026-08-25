use std::time::Duration;

use crate::SandboxAlertShutdownError;

pub trait SandboxAlertLifecyclePort: Send {
    fn shutdown(&mut self, timeout: Duration) -> Result<(), SandboxAlertShutdownError>;
}
