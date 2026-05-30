//! Payload-write contracts.

use model_core::payload::PayloadSegment;

use crate::WriteError;

pub trait PayloadWriteStore {
    fn append_payload_segment(&mut self, segment: PayloadSegment) -> Result<(), WriteError>;
}
