//! Versioned, bounded binary framing for Guest-local sandbox control.

use sandbox_control::{
    MAX_SANDBOX_CONTROL_REJECTION_REASON_BYTES, SandboxConnectCommand, SandboxConnectResponse,
    SandboxControlCommand, SandboxControlRejection, SandboxControlRejectionCode,
    SandboxControlResponse, SandboxEndpoint,
};

use crate::{SandboxControlUdsError, SandboxControlUdsStage};

const MAGIC: [u8; 4] = *b"ASBC";
const VERSION: u8 = 1;
const HEADER_BYTES: usize = 8;
const CONNECT_REQUEST: u8 = 1;
const CONNECT_SUCCESS: u8 = 2;
const REJECTION: u8 = 3;
const CONNECT_PAYLOAD_BYTES: usize = 8;
const SUCCESS_PAYLOAD_BYTES: usize = 21;
const MAX_REJECTION_FRAME_BYTES: usize =
    HEADER_BYTES + 3 + MAX_SANDBOX_CONTROL_REJECTION_REASON_BYTES;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SandboxControlCodec {
    max_frame_bytes: usize,
}

impl SandboxControlCodec {
    pub fn new(max_frame_bytes: usize) -> Result<Self, SandboxControlUdsError> {
        if max_frame_bytes < MAX_REJECTION_FRAME_BYTES
            || max_frame_bytes > HEADER_BYTES + u16::MAX as usize
        {
            return Err(SandboxControlUdsError::new(
                SandboxControlUdsStage::Configure,
                "sandbox control frame limit is outside the binary protocol range",
            ));
        }
        Ok(Self { max_frame_bytes })
    }

    pub const fn max_frame_bytes(&self) -> usize {
        self.max_frame_bytes
    }

    pub fn encode_command(
        &self,
        command: &SandboxControlCommand,
    ) -> Result<Vec<u8>, SandboxControlUdsError> {
        match command {
            SandboxControlCommand::Connect(command) => {
                let endpoint = command.endpoint();
                let mut payload = Vec::with_capacity(CONNECT_PAYLOAD_BYTES);
                payload.extend_from_slice(&endpoint.host_cid().to_be_bytes());
                payload.extend_from_slice(&endpoint.port().to_be_bytes());
                self.encode_frame(CONNECT_REQUEST, &payload)
            }
        }
    }

    pub fn decode_command(
        &self,
        frame: &[u8],
    ) -> Result<SandboxControlCommand, SandboxControlUdsError> {
        let (kind, payload) = self.decode_frame(frame)?;
        if kind != CONNECT_REQUEST || payload.len() != CONNECT_PAYLOAD_BYTES {
            return Err(self.decode_error("expected a fixed-size Connect request"));
        }
        let endpoint = SandboxEndpoint::new(read_u32(payload, 0), read_u32(payload, 4))
            .map_err(|error| self.decode_error(error.to_string()))?;
        Ok(SandboxControlCommand::Connect(SandboxConnectCommand::new(
            endpoint,
        )))
    }

    pub fn encode_response(
        &self,
        response: &SandboxControlResponse,
    ) -> Result<Vec<u8>, SandboxControlUdsError> {
        match response {
            SandboxControlResponse::Connect(response) => self.encode_success(response),
            SandboxControlResponse::Rejected(rejection) => self.encode_rejection(rejection),
        }
    }

    pub fn decode_response(
        &self,
        frame: &[u8],
    ) -> Result<SandboxControlResponse, SandboxControlUdsError> {
        let (kind, payload) = self.decode_frame(frame)?;
        match kind {
            CONNECT_SUCCESS => self.decode_success(payload),
            REJECTION => self.decode_rejection(payload),
            _ => Err(self.decode_error("expected a Connect response")),
        }
    }

    pub(crate) fn frame_len(&self, header: &[u8]) -> Result<Option<usize>, SandboxControlUdsError> {
        if header.len() < HEADER_BYTES {
            return Ok(None);
        }
        if header[..4] != MAGIC || header[4] != VERSION {
            return Err(self.decode_error("invalid sandbox control frame header"));
        }
        let payload_bytes = u16::from_be_bytes([header[6], header[7]]) as usize;
        let frame_bytes = HEADER_BYTES + payload_bytes;
        if frame_bytes > self.max_frame_bytes {
            return Err(self.decode_error("sandbox control frame exceeds configured limit"));
        }
        Ok(Some(frame_bytes))
    }

    fn encode_success(
        &self,
        response: &SandboxConnectResponse,
    ) -> Result<Vec<u8>, SandboxControlUdsError> {
        if response.sb_id() == 0 || response.connection_generation() == 0 {
            return Err(self.encode_error("connected response requires non-zero session identity"));
        }
        let endpoint = response.endpoint();
        let mut payload = Vec::with_capacity(SUCCESS_PAYLOAD_BYTES);
        payload.extend_from_slice(&endpoint.host_cid().to_be_bytes());
        payload.extend_from_slice(&endpoint.port().to_be_bytes());
        payload.extend_from_slice(&response.sb_id().to_be_bytes());
        payload.extend_from_slice(&response.connection_generation().to_be_bytes());
        payload.push(u8::from(response.reused()));
        self.encode_frame(CONNECT_SUCCESS, &payload)
    }

    fn decode_success(
        &self,
        payload: &[u8],
    ) -> Result<SandboxControlResponse, SandboxControlUdsError> {
        if payload.len() != SUCCESS_PAYLOAD_BYTES || payload[20] > 1 {
            return Err(self.decode_error("invalid Connect success payload"));
        }
        let endpoint = SandboxEndpoint::new(read_u32(payload, 0), read_u32(payload, 4))
            .map_err(|error| self.decode_error(error.to_string()))?;
        let sb_id = read_u32(payload, 8);
        let connection_generation = read_u64(payload, 12);
        if sb_id == 0 || connection_generation == 0 {
            return Err(self.decode_error("connected response has zero session identity"));
        }
        Ok(SandboxControlResponse::Connect(
            SandboxConnectResponse::new(endpoint, sb_id, connection_generation, payload[20] == 1),
        ))
    }

    fn encode_rejection(
        &self,
        rejection: &SandboxControlRejection,
    ) -> Result<Vec<u8>, SandboxControlUdsError> {
        let reason = rejection.message().as_bytes();
        let reason_len = u16::try_from(reason.len())
            .map_err(|_| self.encode_error("rejection reason length overflow"))?;
        let mut payload = Vec::with_capacity(3 + reason.len());
        payload.push(encode_rejection_code(rejection.code()));
        payload.extend_from_slice(&reason_len.to_be_bytes());
        payload.extend_from_slice(reason);
        self.encode_frame(REJECTION, &payload)
    }

    fn decode_rejection(
        &self,
        payload: &[u8],
    ) -> Result<SandboxControlResponse, SandboxControlUdsError> {
        if payload.len() < 4 {
            return Err(self.decode_error("rejection payload is truncated"));
        }
        let reason_len = u16::from_be_bytes([payload[1], payload[2]]) as usize;
        if reason_len == 0
            || reason_len > MAX_SANDBOX_CONTROL_REJECTION_REASON_BYTES
            || payload.len() != 3 + reason_len
        {
            return Err(self.decode_error("rejection reason length is invalid"));
        }
        let reason = std::str::from_utf8(&payload[3..]).map_err(|error| {
            self.decode_error(format!("rejection reason is not UTF-8: {error}"))
        })?;
        let rejection = SandboxControlRejection::new(decode_rejection_code(payload[0])?, reason)
            .map_err(|error| self.decode_error(error.to_string()))?;
        Ok(SandboxControlResponse::Rejected(rejection))
    }

    fn encode_frame(&self, kind: u8, payload: &[u8]) -> Result<Vec<u8>, SandboxControlUdsError> {
        let frame_bytes = HEADER_BYTES + payload.len();
        if frame_bytes > self.max_frame_bytes {
            return Err(self.encode_error("sandbox control frame exceeds configured limit"));
        }
        let payload_len = u16::try_from(payload.len())
            .map_err(|_| self.encode_error("sandbox control payload length overflow"))?;
        let mut frame = Vec::with_capacity(frame_bytes);
        frame.extend_from_slice(&MAGIC);
        frame.push(VERSION);
        frame.push(kind);
        frame.extend_from_slice(&payload_len.to_be_bytes());
        frame.extend_from_slice(payload);
        Ok(frame)
    }

    fn decode_frame<'a>(&self, frame: &'a [u8]) -> Result<(u8, &'a [u8]), SandboxControlUdsError> {
        let expected = self
            .frame_len(frame)?
            .ok_or_else(|| self.decode_error("sandbox control frame is truncated"))?;
        if frame.len() != expected {
            return Err(self.decode_error("sandbox control frame length does not match header"));
        }
        Ok((frame[5], &frame[HEADER_BYTES..]))
    }

    fn encode_error(&self, message: impl Into<String>) -> SandboxControlUdsError {
        SandboxControlUdsError::new(SandboxControlUdsStage::Encode, message)
    }

    fn decode_error(&self, message: impl Into<String>) -> SandboxControlUdsError {
        SandboxControlUdsError::new(SandboxControlUdsStage::Decode, message)
    }
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("validated field"),
    )
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_be_bytes(
        bytes[offset..offset + 8]
            .try_into()
            .expect("validated field"),
    )
}

fn encode_rejection_code(code: SandboxControlRejectionCode) -> u8 {
    match code {
        SandboxControlRejectionCode::InvalidRequest => 1,
        SandboxControlRejectionCode::Busy => 2,
        SandboxControlRejectionCode::ConnectFailed => 3,
        SandboxControlRejectionCode::HandshakeFailed => 4,
        SandboxControlRejectionCode::ShuttingDown => 5,
    }
}

fn decode_rejection_code(code: u8) -> Result<SandboxControlRejectionCode, SandboxControlUdsError> {
    match code {
        1 => Ok(SandboxControlRejectionCode::InvalidRequest),
        2 => Ok(SandboxControlRejectionCode::Busy),
        3 => Ok(SandboxControlRejectionCode::ConnectFailed),
        4 => Ok(SandboxControlRejectionCode::HandshakeFailed),
        5 => Ok(SandboxControlRejectionCode::ShuttingDown),
        _ => Err(SandboxControlUdsError::new(
            SandboxControlUdsStage::Decode,
            "unknown sandbox control rejection code",
        )),
    }
}
