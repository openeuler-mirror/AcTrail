//! File access projection from file syscall events.

#[path = "file/projection.rs"]
mod projection;
#[path = "file/shared.rs"]
mod shared;

pub(super) use projection::FileAccessProjector;
