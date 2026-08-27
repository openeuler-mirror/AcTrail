use crate::StoredSandboxAlert;

pub trait SandboxAlertCommitPort: Send + Sync + 'static {
    fn committed(&self, alert: StoredSandboxAlert);
}
