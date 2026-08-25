use crate::frame::HEADER_BYTES;
use crate::{Frame, FrameHeader, WireError};

#[derive(Debug)]
pub struct FrameDecoder {
    buffer: Vec<u8>,
    read_offset: usize,
}

impl FrameDecoder {
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

    pub fn next_frame(&mut self) -> Result<Option<Frame>, WireError> {
        let available = &self.buffer[self.read_offset..];
        if available.len() < HEADER_BYTES {
            return Ok(None);
        }
        let header_bytes: &[u8; HEADER_BYTES] = available[..HEADER_BYTES]
            .try_into()
            .expect("checked header length");
        let header = FrameHeader::decode(header_bytes)?;
        let frame_length = HEADER_BYTES + header.payload_length as usize;
        if available.len() < frame_length {
            return Ok(None);
        }
        let payload = available[HEADER_BYTES..frame_length].to_vec();
        self.read_offset += frame_length;
        Ok(Some(Frame {
            code: header.code,
            payload,
        }))
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
