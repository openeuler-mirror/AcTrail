//! Incremental WebSocket frame, message, and compression assembly.

use flate2::{Decompress, FlushDecompress, Status};

pub(super) const MAX_FRAME_BUFFER_BYTES: usize = 8 * 1024 * 1024;
const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;
pub(super) const MAX_DECODED_BYTES: usize = 32 * 1024 * 1024;
const COMPACT_AFTER_BYTES: usize = 64 * 1024;
const DEFLATE_TAIL: &[u8] = &[0x00, 0x00, 0xff, 0xff];

pub(super) struct DirectionAssembler {
    frames: FrameDecoder,
    message: MessageAssembler,
}

pub(super) struct DirectionObservation {
    pub(super) messages: Vec<Vec<u8>>,
    pub(super) closed: bool,
}

impl DirectionAssembler {
    pub(super) fn new(
        expected_masked: bool,
        deflate_enabled: bool,
        no_context_takeover: bool,
    ) -> Self {
        Self {
            frames: FrameDecoder::new(expected_masked),
            message: MessageAssembler::new(deflate_enabled, no_context_takeover),
        }
    }

    pub(super) fn push(&mut self, bytes: &[u8]) -> Result<DirectionObservation, ()> {
        let mut messages = Vec::new();
        let mut closed = false;
        for frame in self.frames.push(bytes)? {
            closed |= matches!(frame.opcode, Opcode::Close);
            if let Some(message) = self.message.push(frame)? {
                messages.push(message);
            }
        }
        Ok(DirectionObservation { messages, closed })
    }
}

#[derive(Clone, Copy)]
enum Opcode {
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

    fn is_control(self) -> bool {
        matches!(self, Self::Close | Self::Ping | Self::Pong)
    }
}

struct WebSocketFrame {
    fin: bool,
    compressed: bool,
    opcode: Opcode,
    payload: Vec<u8>,
}

pub(super) struct FrameDecoder {
    expected_masked: bool,
    buffer: Vec<u8>,
    cursor: usize,
}

impl FrameDecoder {
    fn new(expected_masked: bool) -> Self {
        Self {
            expected_masked,
            buffer: Vec::new(),
            cursor: 0,
        }
    }

    pub(super) fn looks_like_frame(bytes: &[u8], expected_masked: bool) -> bool {
        let Some((&first, rest)) = bytes.split_first() else {
            return false;
        };
        let Some(&second) = rest.first() else {
            return false;
        };
        first & 0x30 == 0
            && Opcode::parse(first & 0x0f).is_some()
            && (second & 0x80 != 0) == expected_masked
    }

    fn push(&mut self, bytes: &[u8]) -> Result<Vec<WebSocketFrame>, ()> {
        if self
            .buffer
            .len()
            .saturating_sub(self.cursor)
            .saturating_add(bytes.len())
            > MAX_FRAME_BUFFER_BYTES
        {
            self.clear();
            return Err(());
        }
        self.compact_if_needed();
        self.buffer.extend_from_slice(bytes);
        let mut frames = Vec::new();
        while let Some(frame) = self.take_frame()? {
            frames.push(frame);
        }
        self.compact_if_needed();
        Ok(frames)
    }

    fn take_frame(&mut self) -> Result<Option<WebSocketFrame>, ()> {
        let bytes = &self.buffer[self.cursor..];
        if bytes.len() < 2 {
            return Ok(None);
        }
        let first = bytes[0];
        let second = bytes[1];
        let Some(opcode) = Opcode::parse(first & 0x0f) else {
            return self.fail();
        };
        let fin = first & 0x80 != 0;
        let compressed = first & 0x40 != 0;
        let masked = second & 0x80 != 0;
        if first & 0x30 != 0
            || masked != self.expected_masked
            || (opcode.is_control() && (!fin || compressed))
        {
            return self.fail();
        }
        let mut header_len = 2usize;
        let mut payload_len = usize::from(second & 0x7f);
        if payload_len == 126 {
            if bytes.len() < 4 {
                return Ok(None);
            }
            payload_len = usize::from(u16::from_be_bytes([bytes[2], bytes[3]]));
            header_len = 4;
        } else if payload_len == 127 {
            if bytes.len() < 10 {
                return Ok(None);
            }
            let encoded = u64::from_be_bytes(bytes[2..10].try_into().map_err(|_| ())?);
            if encoded & (1 << 63) != 0 {
                return self.fail();
            }
            payload_len = usize::try_from(encoded).map_err(|_| ())?;
            header_len = 10;
        }
        if opcode.is_control() && payload_len > 125 {
            return self.fail();
        }
        let mask = if masked {
            if bytes.len() < header_len + 4 {
                return Ok(None);
            }
            let mask: [u8; 4] = bytes[header_len..header_len + 4]
                .try_into()
                .map_err(|_| ())?;
            header_len += 4;
            Some(mask)
        } else {
            None
        };
        let frame_len = header_len.checked_add(payload_len).ok_or(())?;
        if frame_len > MAX_FRAME_BUFFER_BYTES {
            return self.fail();
        }
        if bytes.len() < frame_len {
            return Ok(None);
        }
        let mut payload = bytes[header_len..frame_len].to_vec();
        if let Some(mask) = mask {
            for (index, byte) in payload.iter_mut().enumerate() {
                *byte ^= mask[index % 4];
            }
        }
        self.cursor += frame_len;
        Ok(Some(WebSocketFrame {
            fin,
            compressed,
            opcode,
            payload,
        }))
    }

    fn compact_if_needed(&mut self) {
        if self.cursor == self.buffer.len() {
            self.clear();
        } else if self.cursor >= COMPACT_AFTER_BYTES && self.cursor >= self.buffer.len() / 2 {
            self.buffer.copy_within(self.cursor.., 0);
            self.buffer.truncate(self.buffer.len() - self.cursor);
            self.cursor = 0;
        }
    }

    fn clear(&mut self) {
        self.buffer.clear();
        self.cursor = 0;
    }

    fn fail<T>(&mut self) -> Result<T, ()> {
        self.clear();
        Err(())
    }
}

struct FragmentedMessage {
    opcode: Opcode,
    compressed: bool,
    payload: Vec<u8>,
}

struct MessageAssembler {
    fragment: Option<FragmentedMessage>,
    deflate: PerMessageDeflateDecoder,
}

impl MessageAssembler {
    fn new(deflate_enabled: bool, no_context_takeover: bool) -> Self {
        Self {
            fragment: None,
            deflate: PerMessageDeflateDecoder::new(deflate_enabled, no_context_takeover),
        }
    }

    fn push(&mut self, frame: WebSocketFrame) -> Result<Option<Vec<u8>>, ()> {
        if frame.opcode.is_control() {
            return Ok(None);
        }
        match frame.opcode {
            Opcode::Text | Opcode::Binary => {
                if self.fragment.is_some() || frame.payload.len() > MAX_MESSAGE_BYTES {
                    return self.fail();
                }
                if frame.fin {
                    return self.complete(frame.compressed, frame.payload).map(Some);
                }
                self.fragment = Some(FragmentedMessage {
                    opcode: frame.opcode,
                    compressed: frame.compressed,
                    payload: frame.payload,
                });
                Ok(None)
            }
            Opcode::Continuation => {
                if frame.compressed {
                    return self.fail();
                }
                let Some(mut fragment) = self.fragment.take() else {
                    return self.fail();
                };
                if fragment.payload.len().saturating_add(frame.payload.len()) > MAX_MESSAGE_BYTES {
                    return self.fail();
                }
                fragment.payload.extend_from_slice(&frame.payload);
                if !frame.fin {
                    self.fragment = Some(fragment);
                    return Ok(None);
                }
                let _opcode = fragment.opcode;
                self.complete(fragment.compressed, fragment.payload)
                    .map(Some)
            }
            Opcode::Close | Opcode::Ping | Opcode::Pong => Ok(None),
        }
    }

    fn complete(&mut self, compressed: bool, payload: Vec<u8>) -> Result<Vec<u8>, ()> {
        if compressed {
            self.deflate.decode(&payload)
        } else {
            Ok(payload)
        }
    }

    fn fail<T>(&mut self) -> Result<T, ()> {
        self.fragment = None;
        Err(())
    }
}

struct PerMessageDeflateDecoder {
    enabled: bool,
    no_context_takeover: bool,
    decoder: Decompress,
}

impl PerMessageDeflateDecoder {
    fn new(enabled: bool, no_context_takeover: bool) -> Self {
        Self {
            enabled,
            no_context_takeover,
            decoder: Decompress::new(false),
        }
    }

    fn decode(&mut self, payload: &[u8]) -> Result<Vec<u8>, ()> {
        if !self.enabled {
            return Err(());
        }
        let mut input = Vec::with_capacity(payload.len() + DEFLATE_TAIL.len());
        input.extend_from_slice(payload);
        input.extend_from_slice(DEFLATE_TAIL);
        let mut cursor = 0usize;
        let mut output = Vec::new();
        loop {
            let mut chunk = [0u8; 4096];
            let before_in = self.decoder.total_in();
            let before_out = self.decoder.total_out();
            let status = self
                .decoder
                .decompress(&input[cursor..], &mut chunk, FlushDecompress::Sync)
                .map_err(|_| ())?;
            let consumed = usize::try_from(self.decoder.total_in() - before_in).map_err(|_| ())?;
            let produced =
                usize::try_from(self.decoder.total_out() - before_out).map_err(|_| ())?;
            cursor = cursor.checked_add(consumed).ok_or(())?;
            if output.len().saturating_add(produced) > MAX_DECODED_BYTES {
                return Err(());
            }
            output.extend_from_slice(&chunk[..produced]);
            if cursor == input.len() && produced < chunk.len() {
                break;
            }
            if consumed == 0 && produced == 0 {
                if cursor == input.len() && status == Status::BufError {
                    break;
                }
                return Err(());
            }
        }
        if cursor != input.len() {
            return Err(());
        }
        if self.no_context_takeover {
            self.decoder.reset(false);
        }
        Ok(output)
    }
}
