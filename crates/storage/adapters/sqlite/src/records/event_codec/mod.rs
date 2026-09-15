//! Event payload codec: split large fields + pluggable binary serializer.
//!
//! `EventPayload` splits into two parts on write:
//! - large fields (`ApplicationPayload.body`, `StdioPayload.data`) become
//!   `PayloadBlock`s, compressed with zstd and stored in a side table;
//! - the remaining small fields serialize via a swappable `EventPayloadCodec`
//!   (bincode today) into `EncodedEventPayload.fields`.
//!
//! The codec is an explicit seam so a hand-written binary format can replace
//! bincode without touching the storage schema, block split, or call sites.

use model_core::event::EventPayload;

mod bincode_codec;
mod blocks;
mod key_codes;
mod manual;
mod value_codes;

pub use blocks::{join_large_fields, split_large_fields};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlockKind {
    HttpBodyText = 0,
    HttpBodyJson = 1,
    HttpBodyBase64 = 2,
    StdioData = 3,
}

impl BlockKind {
    pub fn to_i64(self) -> i64 {
        self as i64
    }

    pub fn from_i64(value: i64) -> Option<Self> {
        match value {
            0 => Some(Self::HttpBodyText),
            1 => Some(Self::HttpBodyJson),
            2 => Some(Self::HttpBodyBase64),
            3 => Some(Self::StdioData),
            _ => None,
        }
    }
}

pub struct PayloadBlock {
    pub kind: BlockKind,
    pub bytes: Vec<u8>,
}

pub struct EncodedEventPayload {
    pub fields: Vec<u8>,
    pub blocks: Vec<PayloadBlock>,
}

pub trait EventPayloadCodec: Send + Sync {
    fn encode(&self, payload: &EventPayload) -> Result<Vec<u8>, String>;
    fn decode(&self, bytes: &[u8]) -> Result<EventPayload, String>;
}

pub fn event_payload_codec() -> &'static dyn EventPayloadCodec {
    static CODEC: manual::ManualCodec = manual::ManualCodec;
    &CODEC
}
