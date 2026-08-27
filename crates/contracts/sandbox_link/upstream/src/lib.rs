//! Compact gateway-to-daemon wire contract.

mod error;
mod frame;
mod stream;

pub use error::WireError;
pub use frame::{ForwardedSbFrame, Frame, FrameCode, FrameHeader, MAX_FRAME_BYTES};
pub use stream::FrameDecoder;
