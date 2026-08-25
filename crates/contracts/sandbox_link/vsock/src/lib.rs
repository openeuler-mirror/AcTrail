//! Compact SB-to-gateway wire contract.

mod batch_codec;
mod error;
mod frame;
mod stream;

pub use batch_codec::ObservationBatchCodec;
pub use error::WireError;
pub use frame::{Frame, FrameCode, FrameHeader, MAX_FRAME_BYTES};
pub use stream::FrameDecoder;
