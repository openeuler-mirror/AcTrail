//! Shared TLS payload decision types.

mod direction;
mod error;
mod processor;
mod rule;

pub use direction::PayloadDirection;
pub use error::{CoreError, CoreResult};
pub use processor::{Decision, PayloadContext, SyncProcessor};
pub use rule::{EqualLenRewriteProcessor, RewriteRule};
