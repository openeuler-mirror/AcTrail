//! Authoritative agent task, interaction and work lifecycle signals.
mod model;
pub use model::{
    SessionClosedEvent, TurnLifecycleEvent, TurnLifecycleKind, UserInteractionEvent,
    UserInteractionState, UserLifecycleEvent, WorkKind, WorkLifecycleEvent, WorkState,
};
