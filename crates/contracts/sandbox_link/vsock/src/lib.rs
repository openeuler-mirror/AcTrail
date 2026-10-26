//! Compact SB-to-gateway wire contract.

mod batch_codec;
mod error;
mod frame;
mod stream;

pub use batch_codec::{
    MAX_ENCODED_OBSERVATION_BYTES, OBSERVATION_BATCH_FIXED_BYTES, ObservationBatchCodec,
};
pub use error::WireError;
pub use frame::{Frame, FrameCode, FrameHeader, HEADER_BYTES, MAX_FRAME_BYTES, SbWelcome};
pub use stream::FrameDecoder;
