//! Per-message DEFLATE decoding with optional context takeover.

use flate2::{Decompress, FlushDecompress, Status};

use crate::{ToolError, ToolResult};

const DEFLATE_TAIL: &[u8] = &[0x00, 0x00, 0xff, 0xff];
const OUTPUT_CHUNK_BYTES: usize = 4_096;

#[derive(Debug)]
pub(super) struct PerMessageDeflateDecoder {
    enabled: bool,
    no_context_takeover: bool,
    max_output_bytes: usize,
    decoder: Decompress,
}

impl PerMessageDeflateDecoder {
    pub(super) fn new(enabled: bool, no_context_takeover: bool, max_output_bytes: usize) -> Self {
        Self {
            enabled,
            no_context_takeover,
            max_output_bytes,
            decoder: Decompress::new(false),
        }
    }

    pub(super) fn decode(&mut self, payload: &[u8]) -> ToolResult<Vec<u8>> {
        if !self.enabled {
            return Err(ToolError::new(
                "WebSocket message uses RSV1 without negotiated permessage-deflate",
            ));
        }
        let mut input = Vec::with_capacity(payload.len() + DEFLATE_TAIL.len());
        input.extend_from_slice(payload);
        input.extend_from_slice(DEFLATE_TAIL);
        let mut cursor = 0usize;
        let mut output = Vec::new();
        loop {
            let mut chunk = [0u8; OUTPUT_CHUNK_BYTES];
            let before_in = self.decoder.total_in();
            let before_out = self.decoder.total_out();
            let status = self
                .decoder
                .decompress(&input[cursor..], &mut chunk, FlushDecompress::Sync)
                .map_err(|error| ToolError::new(format!("WebSocket deflate failed: {error}")))?;
            let consumed = usize::try_from(self.decoder.total_in() - before_in)
                .map_err(|_| ToolError::new("WebSocket deflate input count exceeds usize"))?;
            let produced = usize::try_from(self.decoder.total_out() - before_out)
                .map_err(|_| ToolError::new("WebSocket deflate output count exceeds usize"))?;
            cursor = cursor
                .checked_add(consumed)
                .ok_or_else(|| ToolError::new("WebSocket deflate input offset overflow"))?;
            let next_len = output
                .len()
                .checked_add(produced)
                .ok_or_else(|| ToolError::new("WebSocket decoded length overflow"))?;
            if next_len > self.max_output_bytes {
                return Err(ToolError::new(format!(
                    "WebSocket decoded message exceeded {} bytes",
                    self.max_output_bytes
                )));
            }
            output.extend_from_slice(&chunk[..produced]);
            if cursor == input.len() && produced < chunk.len() {
                break;
            }
            if consumed == 0 && produced == 0 {
                if cursor == input.len() && status == Status::BufError {
                    break;
                }
                return Err(ToolError::new("WebSocket deflate made no progress"));
            }
        }
        if cursor != input.len() {
            return Err(ToolError::new("WebSocket deflate left unconsumed input"));
        }
        if self.no_context_takeover {
            self.decoder.reset(false);
        }
        Ok(output)
    }
}
