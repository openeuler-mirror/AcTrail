use crate::{SandboxAlertAdmission, SandboxAlertRecord};

pub trait SandboxAlertWritePort: Send + Sync + 'static {
    fn try_append(&self, alert: SandboxAlertRecord) -> SandboxAlertAdmission;
}
