use crate::{NoInterestEvidenceBatch, SandboxEvidenceAdmission};

pub trait SandboxEvidenceWritePort: Send + Sync + 'static {
    /// Attempts bounded admission. Implementations must not perform storage I/O on this call.
    fn try_append_batch(&self, batch: NoInterestEvidenceBatch) -> SandboxEvidenceAdmission;
}
