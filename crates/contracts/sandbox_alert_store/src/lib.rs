//! Narrow ports and typed records for the independent sandbox alert store.

mod commit;
mod lifecycle;
mod read;
mod record;
mod result;
mod status;
mod write;

pub use commit::SandboxAlertCommitPort;
pub use lifecycle::SandboxAlertLifecyclePort;
pub use read::SandboxAlertReadPort;
pub use record::{SandboxAlertKind, SandboxAlertRecord, SandboxAlertSource, StoredSandboxAlert};
pub use result::{
    SandboxAlertAdmission, SandboxAlertReadError, SandboxAlertShutdownError,
    SandboxAlertSourceError,
};
pub use status::{SandboxAlertHealth, SandboxAlertStatus, SandboxAlertStatusPort};
pub use write::SandboxAlertWritePort;
