//! Guest-local observation contracts for the isolated hand-side data path.

mod observation;
mod process;
mod resource;

pub use observation::{Observation, ObservationBatch};
pub use process::{GuestBootId, ProcessIoCounters, ProcessMarker};
pub use resource::{CpuSnapshot, GuestResourceSnapshot, MemorySnapshot};
