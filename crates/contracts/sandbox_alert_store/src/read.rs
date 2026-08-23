use crate::{SandboxAlertReadError, StoredSandboxAlert};

pub trait SandboxAlertReadPort: Send + Sync + 'static {
    fn recent(&self, limit: u32) -> Result<Vec<StoredSandboxAlert>, SandboxAlertReadError>;
}
