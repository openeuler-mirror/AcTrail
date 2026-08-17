//! bincode-backed `EventPayloadCodec`.
//!
//! Kept as a swappable alternative to the default hand-written codec.

use model_core::event::EventPayload;

use super::EventPayloadCodec;

#[allow(dead_code)]
pub struct BincodeCodec;

impl EventPayloadCodec for BincodeCodec {
    fn encode(&self, payload: &EventPayload) -> Result<Vec<u8>, String> {
        bincode::serialize(payload).map_err(|error| format!("bincode encode: {error}"))
    }

    fn decode(&self, bytes: &[u8]) -> Result<EventPayload, String> {
        bincode::deserialize(bytes).map_err(|error| format!("bincode decode: {error}"))
    }
}
