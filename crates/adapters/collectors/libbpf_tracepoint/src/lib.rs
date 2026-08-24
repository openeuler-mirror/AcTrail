//! Shared standard-tracepoint attachment policy for libbpf collectors.

mod attacher;
mod error;

pub use attacher::{TracepointAttachOutcome, TracepointProgramAttacher, TracepointRequirement};
pub use error::TracepointAttachError;
