use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::DeliveryCodecError;

const JSON_FRAME_HEADER_BYTES: usize = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JsonFrameCodec {
    max_payload_bytes: usize,
}

impl JsonFrameCodec {
    pub fn new(max_payload_bytes: usize) -> Result<Self, DeliveryCodecError> {
        if max_payload_bytes == 0 || max_payload_bytes > u32::MAX as usize {
            return Err(DeliveryCodecError::new(
                "json_frame_limits",
                format!("max payload bytes must be between 1 and {}", u32::MAX),
            ));
        }
        Ok(Self { max_payload_bytes })
    }

    pub const fn max_payload_bytes(self) -> usize {
        self.max_payload_bytes
    }

    pub fn encode<T>(&self, message: &T) -> Result<Vec<u8>, DeliveryCodecError>
    where
        T: Serialize,
    {
        let payload = serde_json::to_vec(message).map_err(|error| {
            DeliveryCodecError::new("json_frame_encode", format!("serialize JSON: {error}"))
        })?;
        if payload.len() > self.max_payload_bytes {
            return Err(DeliveryCodecError::new(
                "json_frame_encode",
                format!(
                    "JSON payload length {} exceeds configured limit {}",
                    payload.len(),
                    self.max_payload_bytes
                ),
            ));
        }
        let payload_length = u32::try_from(payload.len()).map_err(|_| {
            DeliveryCodecError::new("json_frame_encode", "JSON payload length does not fit u32")
        })?;
        let mut frame = Vec::with_capacity(JSON_FRAME_HEADER_BYTES + payload.len());
        frame.extend_from_slice(&payload_length.to_be_bytes());
        frame.extend_from_slice(&payload);
        Ok(frame)
    }

    pub fn decode<T>(&self, payload: &[u8]) -> Result<T, DeliveryCodecError>
    where
        T: DeserializeOwned,
    {
        if payload.len() > self.max_payload_bytes {
            return Err(DeliveryCodecError::new(
                "json_frame_decode",
                format!(
                    "JSON payload length {} exceeds configured limit {}",
                    payload.len(),
                    self.max_payload_bytes
                ),
            ));
        }
        serde_json::from_slice(payload).map_err(|error| {
            DeliveryCodecError::new("json_frame_decode", format!("decode JSON: {error}"))
        })
    }

    fn payload_length(&self, header: &[u8]) -> Result<usize, DeliveryCodecError> {
        let bytes: &[u8; JSON_FRAME_HEADER_BYTES] = header.try_into().map_err(|_| {
            DeliveryCodecError::new(
                "json_frame_decode",
                "JSON frame header must contain exactly four bytes",
            )
        })?;
        let length = u32::from_be_bytes(*bytes) as usize;
        if length > self.max_payload_bytes {
            return Err(DeliveryCodecError::new(
                "json_frame_decode",
                format!(
                    "JSON payload length {length} exceeds configured limit {}",
                    self.max_payload_bytes
                ),
            ));
        }
        Ok(length)
    }
}

#[derive(Debug)]
pub struct JsonFrameDecoder {
    buffer: Vec<u8>,
    read_offset: usize,
}

impl JsonFrameDecoder {
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

    pub fn next<T>(&mut self, codec: &JsonFrameCodec) -> Result<Option<T>, DeliveryCodecError>
    where
        T: DeserializeOwned,
    {
        let available = &self.buffer[self.read_offset..];
        if available.len() < JSON_FRAME_HEADER_BYTES {
            return Ok(None);
        }
        let payload_length = codec.payload_length(&available[..JSON_FRAME_HEADER_BYTES])?;
        let frame_length = JSON_FRAME_HEADER_BYTES
            .checked_add(payload_length)
            .ok_or_else(|| {
                DeliveryCodecError::new("json_frame_decode", "JSON frame length overflow")
            })?;
        if available.len() < frame_length {
            return Ok(None);
        }
        let message = codec.decode(&available[JSON_FRAME_HEADER_BYTES..frame_length])?;
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
