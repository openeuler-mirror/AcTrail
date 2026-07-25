//! Incremental WebSocket wire-frame decoding.

use crate::{ToolError, ToolResult};

const MASK_BIT: u8 = 0x80;
const LENGTH_MASK: u8 = 0x7f;
const RSV1_BIT: u8 = 0x40;
const RSV23_MASK: u8 = 0x30;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Opcode {
    Continuation,
    Text,
    Binary,
    Close,
    Ping,
    Pong,
}

impl Opcode {
    fn parse(value: u8) -> Option<Self> {
        match value {
            0x0 => Some(Self::Continuation),
            0x1 => Some(Self::Text),
            0x2 => Some(Self::Binary),
            0x8 => Some(Self::Close),
            0x9 => Some(Self::Ping),
            0xa => Some(Self::Pong),
            _ => None,
        }
    }

    pub(super) fn is_control(self) -> bool {
        matches!(self, Self::Close | Self::Ping | Self::Pong)
    }
}

#[derive(Debug)]
pub(super) struct WebSocketFrame {
    pub(super) fin: bool,
    pub(super) compressed: bool,
    pub(super) opcode: Opcode,
    pub(super) wire_bytes: usize,
    pub(super) payload: Vec<u8>,
}

#[derive(Debug)]
pub(super) struct FrameDecoder {
    expected_masked: bool,
    max_buffer_bytes: usize,
    buffer: Vec<u8>,
}

impl FrameDecoder {
    pub(super) fn new(expected_masked: bool, max_buffer_bytes: usize) -> Self {
        Self {
            expected_masked,
            max_buffer_bytes,
            buffer: Vec::new(),
        }
    }

    pub(super) fn looks_like_frame(bytes: &[u8], expected_masked: bool) -> bool {
        let Some((&first, rest)) = bytes.split_first() else {
            return false;
        };
        let Some(&second) = rest.first() else {
            return false;
        };
        if first & RSV23_MASK != 0 || Opcode::parse(first & 0x0f).is_none() {
            return false;
        }
        (second & MASK_BIT != 0) == expected_masked
    }

    pub(super) fn push(&mut self, bytes: &[u8]) -> ToolResult<Vec<WebSocketFrame>> {
        let next_len = self
            .buffer
            .len()
            .checked_add(bytes.len())
            .ok_or_else(|| ToolError::new("WebSocket frame buffer length overflow"))?;
        if next_len > self.max_buffer_bytes {
            self.buffer.clear();
            return Err(ToolError::new(format!(
                "WebSocket frame buffer exceeded {} bytes",
                self.max_buffer_bytes
            )));
        }
        self.buffer.extend_from_slice(bytes);
        let mut frames = Vec::new();
        while let Some(frame) = self.take_frame()? {
            frames.push(frame);
        }
        Ok(frames)
    }

    fn take_frame(&mut self) -> ToolResult<Option<WebSocketFrame>> {
        if self.buffer.len() < 2 {
            return Ok(None);
        }
        let first = self.buffer[0];
        let second = self.buffer[1];
        if first & RSV23_MASK != 0 {
            return self.protocol_error("RSV2 or RSV3 is set");
        }
        let opcode = Opcode::parse(first & 0x0f)
            .ok_or_else(|| ToolError::new("WebSocket frame has an unsupported opcode"))?;
        let fin = first & 0x80 != 0;
        let compressed = first & RSV1_BIT != 0;
        let masked = second & MASK_BIT != 0;
        if masked != self.expected_masked {
            return self.protocol_error("WebSocket frame mask direction is invalid");
        }
        if opcode.is_control() && (!fin || compressed) {
            return self.protocol_error("WebSocket control frame is fragmented or compressed");
        }

        let mut cursor = 2usize;
        let mut payload_len = usize::from(second & LENGTH_MASK);
        if payload_len == 126 {
            if self.buffer.len() < cursor + 2 {
                return Ok(None);
            }
            payload_len = usize::from(u16::from_be_bytes([
                self.buffer[cursor],
                self.buffer[cursor + 1],
            ]));
            cursor += 2;
        } else if payload_len == 127 {
            if self.buffer.len() < cursor + 8 {
                return Ok(None);
            }
            let encoded = u64::from_be_bytes(
                self.buffer[cursor..cursor + 8]
                    .try_into()
                    .map_err(|_| ToolError::new("WebSocket length field is incomplete"))?,
            );
            if encoded & (1 << 63) != 0 {
                return self.protocol_error("WebSocket 64-bit length has its high bit set");
            }
            payload_len = usize::try_from(encoded)
                .map_err(|_| ToolError::new("WebSocket payload length exceeds usize"))?;
            cursor += 8;
        }
        if opcode.is_control() && payload_len > 125 {
            return self.protocol_error("WebSocket control frame exceeds 125 bytes");
        }

        let mask = if masked {
            if self.buffer.len() < cursor + 4 {
                return Ok(None);
            }
            let value = [
                self.buffer[cursor],
                self.buffer[cursor + 1],
                self.buffer[cursor + 2],
                self.buffer[cursor + 3],
            ];
            cursor += 4;
            Some(value)
        } else {
            None
        };
        let frame_len = cursor
            .checked_add(payload_len)
            .ok_or_else(|| ToolError::new("WebSocket frame length overflow"))?;
        if frame_len > self.max_buffer_bytes {
            return self.protocol_error("WebSocket frame exceeds configured buffer limit");
        }
        if self.buffer.len() < frame_len {
            return Ok(None);
        }

        let mut payload = self.buffer[cursor..frame_len].to_vec();
        if let Some(mask) = mask {
            for (index, byte) in payload.iter_mut().enumerate() {
                *byte ^= mask[index % mask.len()];
            }
        }
        self.buffer.drain(..frame_len);
        Ok(Some(WebSocketFrame {
            fin,
            compressed,
            opcode,
            wire_bytes: frame_len,
            payload,
        }))
    }

    fn protocol_error<T>(&mut self, message: &str) -> ToolResult<T> {
        self.buffer.clear();
        Err(ToolError::new(message))
    }
}
