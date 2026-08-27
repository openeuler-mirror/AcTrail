use crate::{DeliveryCodecError, internal::AtapMessage};

use super::codec::AtapCodec;
use super::frame::ATAP_HEADER_BYTES;

#[derive(Debug)]
pub struct AtapStreamDecoder {
    buffer: Vec<u8>,
    read_offset: usize,
}

impl AtapStreamDecoder {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            buffer: Vec::with_capacity(capacity),
            read_offset: 0,
        }
    }

    pub fn push(&mut self, bytes: &[u8]) {
        self.compact_if_needed();
        self.buffer.extend_from_slice(bytes);
    }

    pub fn next_message(
        &mut self,
        codec: &AtapCodec,
    ) -> Result<Option<AtapMessage>, DeliveryCodecError> {
        let available = &self.buffer[self.read_offset..];
        if available.len() < ATAP_HEADER_BYTES {
            return Ok(None);
        }
        let frame_length = codec.frame_length(&available[..ATAP_HEADER_BYTES])?;
        if available.len() < frame_length {
            return Ok(None);
        }
        let message = codec.decode(&available[..frame_length])?;
        self.read_offset += frame_length;
        Ok(Some(message))
    }

    pub fn buffered_bytes(&self) -> usize {
        self.buffer.len().saturating_sub(self.read_offset)
    }

    fn compact_if_needed(&mut self) {
        if self.read_offset == 0 {
            return;
        }
        if self.read_offset == self.buffer.len() {
            self.buffer.clear();
            self.read_offset = 0;
        } else if self.read_offset >= self.buffer.capacity() / 2 {
            self.buffer.drain(..self.read_offset);
            self.read_offset = 0;
        }
    }
}
