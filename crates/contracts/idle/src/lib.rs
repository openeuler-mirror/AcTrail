//! No-observable-progress (idle) interval contracts.
//!
//! These types describe gaps inside an active user task during which AcTrail
//! observed no new execution steps, state changes, or effective output. The
//! detector consumes lifecycle events and real-time semantic action updates
//! and persists `IdleInterval` records independently of alert payloads.
// Public idle contract exports.

mod error;
mod model;
mod store;

pub use error::{IdleStoreError, IdleStoreErrorKind};
pub use model::{
    IdleInterval, IdleIntervalId, TurnLifecycleEvent, TurnLifecycleKind, UserInteractionEvent,
    UserInteractionState,
};
pub use store::{IdleReadStore, IdleStoreOp, IdleWriteStore};
