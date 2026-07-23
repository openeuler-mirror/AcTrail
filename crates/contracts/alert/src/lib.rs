//! Generic plugin-produced alert contracts.

mod error;
mod model;
mod store;

pub use error::{AlertStoreError, AlertStoreErrorKind};
pub use model::{
    AlertDefinition, AlertDefinitionId, AlertDraft, AlertId, AlertListLimit, AlertRecord,
    AlertSeverity, AlertSubmitOutcome, AlertView,
};
pub use store::{AlertDefinitionStore, AlertReadStore, AlertWriteStore};
