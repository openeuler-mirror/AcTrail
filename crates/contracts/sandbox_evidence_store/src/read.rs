use crate::{SandboxEvidenceReadError, StoredSandboxEvidence};

pub trait SandboxEvidenceReadPort: Send + Sync + 'static {
    fn recent(&self, limit: u32) -> Result<Vec<StoredSandboxEvidence>, SandboxEvidenceReadError>;
}
