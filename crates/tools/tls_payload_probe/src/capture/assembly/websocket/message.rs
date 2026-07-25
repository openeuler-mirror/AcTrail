//! WebSocket fragmented-message assembly.

use crate::{ToolError, ToolResult};

use super::deflate::PerMessageDeflateDecoder;
use super::frame::{Opcode, WebSocketFrame};
use super::model::WebSocketPayload;

#[derive(Debug)]
pub(super) struct AssembledMessage {
    pub(super) compressed: bool,
    pub(super) wire_bytes: usize,
    pub(super) payload: WebSocketPayload,
}

#[derive(Debug)]
struct FragmentedMessage {
    opcode: Opcode,
    compressed: bool,
    wire_bytes: usize,
    payload: Vec<u8>,
}

#[derive(Debug)]
pub(super) struct MessageAssembler {
    max_message_bytes: usize,
    fragment: Option<FragmentedMessage>,
    decoder: PerMessageDeflateDecoder,
}

impl MessageAssembler {
    pub(super) fn new(
        max_message_bytes: usize,
        max_decoded_bytes: usize,
        deflate_enabled: bool,
        no_context_takeover: bool,
    ) -> Self {
        Self {
            max_message_bytes,
            fragment: None,
            decoder: PerMessageDeflateDecoder::new(
                deflate_enabled,
                no_context_takeover,
                max_decoded_bytes,
            ),
        }
    }

    pub(super) fn push(&mut self, frame: WebSocketFrame) -> ToolResult<Option<AssembledMessage>> {
        if frame.opcode.is_control() {
            return Ok(None);
        }
        match frame.opcode {
            Opcode::Text | Opcode::Binary => self.start(frame),
            Opcode::Continuation => self.continue_message(frame),
            Opcode::Close | Opcode::Ping | Opcode::Pong => Ok(None),
        }
    }

    fn start(&mut self, frame: WebSocketFrame) -> ToolResult<Option<AssembledMessage>> {
        if self.fragment.is_some() {
            return self.fail("WebSocket data frame arrived before fragmented message completed");
        }
        self.check_message_size(frame.payload.len())?;
        if frame.fin {
            return self.complete(
                frame.opcode,
                frame.compressed,
                frame.wire_bytes,
                frame.payload,
            );
        }
        self.fragment = Some(FragmentedMessage {
            opcode: frame.opcode,
            compressed: frame.compressed,
            wire_bytes: frame.wire_bytes,
            payload: frame.payload,
        });
        Ok(None)
    }

    fn continue_message(&mut self, frame: WebSocketFrame) -> ToolResult<Option<AssembledMessage>> {
        if frame.compressed {
            return self.fail("WebSocket continuation frame has RSV1 set");
        }
        let Some(mut fragment) = self.fragment.take() else {
            return self.fail("WebSocket continuation frame has no initial data frame");
        };
        let next_len = fragment
            .payload
            .len()
            .checked_add(frame.payload.len())
            .ok_or_else(|| ToolError::new("WebSocket fragmented message length overflow"))?;
        self.check_message_size(next_len)?;
        fragment.payload.extend_from_slice(&frame.payload);
        fragment.wire_bytes = fragment
            .wire_bytes
            .checked_add(frame.wire_bytes)
            .ok_or_else(|| ToolError::new("WebSocket wire byte count overflow"))?;
        if !frame.fin {
            self.fragment = Some(fragment);
            return Ok(None);
        }
        self.complete(
            fragment.opcode,
            fragment.compressed,
            fragment.wire_bytes,
            fragment.payload,
        )
    }

    fn complete(
        &mut self,
        opcode: Opcode,
        compressed: bool,
        wire_bytes: usize,
        payload: Vec<u8>,
    ) -> ToolResult<Option<AssembledMessage>> {
        let payload = if compressed {
            self.decoder.decode(&payload)?
        } else {
            payload
        };
        let payload = match opcode {
            Opcode::Text => {
                WebSocketPayload::Text(String::from_utf8(payload).map_err(|error| {
                    ToolError::new(format!("WebSocket text is not UTF-8: {error}"))
                })?)
            }
            Opcode::Binary => WebSocketPayload::Binary(payload),
            Opcode::Continuation | Opcode::Close | Opcode::Ping | Opcode::Pong => {
                return self.fail("WebSocket message completed with an invalid opcode");
            }
        };
        Ok(Some(AssembledMessage {
            compressed,
            wire_bytes,
            payload,
        }))
    }

    fn check_message_size(&self, size: usize) -> ToolResult<()> {
        if size > self.max_message_bytes {
            return Err(ToolError::new(format!(
                "WebSocket compressed message exceeded {} bytes",
                self.max_message_bytes
            )));
        }
        Ok(())
    }

    fn fail<T>(&mut self, message: &str) -> ToolResult<T> {
        self.fragment = None;
        Err(ToolError::new(message))
    }
}
