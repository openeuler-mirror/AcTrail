//! Narrow ports and owned DTOs for the independent sandbox evidence store.

mod lifecycle;
mod read;
mod record;
mod result;
mod status;
mod write;

pub use lifecycle::SandboxEvidenceLifecyclePort;
pub use read::SandboxEvidenceReadPort;
pub use record::{NoInterestEvidenceBatch, SandboxEvidenceSource, StoredSandboxEvidence};
pub use result::{
    SandboxEvidenceAdmission, SandboxEvidenceBatchError, SandboxEvidenceReadError,
    SandboxEvidenceShutdownError, SandboxEvidenceSourceError,
};
pub use sandbox_observation;
pub use status::{SandboxEvidenceHealth, SandboxEvidenceStatus, SandboxEvidenceStatusPort};
pub use write::SandboxEvidenceWritePort;
