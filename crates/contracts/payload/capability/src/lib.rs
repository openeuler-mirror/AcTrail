//! Payload capability contracts and request modes.

mod capability;
mod constants;

pub use capability::PayloadCapability;
pub use constants::{
    DEFAULT_TLS_SYNC_FLOW_UNKNOWN_STREAM_BYTES, DEFAULT_TLS_SYNC_MAX_FRAME_BYTES,
    TLS_SYNC_FRAME_HEADER_BYTES,
};
