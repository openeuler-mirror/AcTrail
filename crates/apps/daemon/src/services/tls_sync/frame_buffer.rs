//! Per-connection binary framing; consumed data is compacted once per read batch.

use tls_payload_sync::{FrameCodec, SyncMessage, SyncResult};

#[derive(Default)]
pub(super) struct FrameBuffer {
    bytes: Vec<u8>,
    consumed: usize,
}

impl FrameBuffer {
    pub(super) fn append(&mut self, bytes: &[u8]) {
        self.bytes.extend_from_slice(bytes);
    }

    pub(super) fn next(&mut self, max_frame_bytes: usize) -> SyncResult<Option<SyncMessage>> {
        let remaining = &self.bytes[self.consumed..];
        let Some(header) = FrameCodec::parse_header(remaining, max_frame_bytes)? else {
            return Ok(None);
        };
        let frame_len = header.frame_len();
        if remaining.len() < frame_len {
            return Ok(None);
        }
        let message = FrameCodec::decode(header, &remaining[FrameCodec::HEADER_LEN..frame_len])?;
        self.consumed += frame_len;
        Ok(Some(message))
    }

    pub(super) fn compact(&mut self) {
        if self.consumed == self.bytes.len() {
            self.bytes.clear();
        } else if self.consumed != 0 {
            self.bytes.copy_within(self.consumed.., 0);
            self.bytes.truncate(self.bytes.len() - self.consumed);
        }
        self.consumed = 0;
    }

    pub(super) fn is_empty(&self) -> bool {
        self.consumed == self.bytes.len()
    }
}
