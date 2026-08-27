use crate::WireError;

const MAGIC: u16 = 0xac72;
const VERSION: u8 = 1;
pub const MAX_FRAME_BYTES: usize = 272 * 1024;
pub const HEADER_BYTES: usize = 8;

#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrameCode {
    GatewayHello = 32,
    GatewayWelcome = 33,
    Heartbeat = 34,
    ForwardedSbFrame = 48,
}

impl TryFrom<u8> for FrameCode {
    type Error = WireError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            32 => Ok(Self::GatewayHello),
            33 => Ok(Self::GatewayWelcome),
            34 => Ok(Self::Heartbeat),
            48 => Ok(Self::ForwardedSbFrame),
            other => Err(WireError::new(format!(
                "unknown upstream frame code {other}"
            ))),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameHeader {
    pub code: FrameCode,
    pub payload_length: u32,
}

impl FrameHeader {
    pub fn encode(self) -> [u8; HEADER_BYTES] {
        let mut bytes = [0_u8; HEADER_BYTES];
        bytes[0..2].copy_from_slice(&MAGIC.to_be_bytes());
        bytes[2] = VERSION;
        bytes[3] = self.code as u8;
        bytes[4..8].copy_from_slice(&self.payload_length.to_be_bytes());
        bytes
    }

    pub fn decode(bytes: &[u8; HEADER_BYTES]) -> Result<Self, WireError> {
        if u16::from_be_bytes([bytes[0], bytes[1]]) != MAGIC {
            return Err(WireError::new("invalid upstream frame magic"));
        }
        if bytes[2] != VERSION {
            return Err(WireError::new(format!(
                "unsupported upstream protocol version {}",
                bytes[2]
            )));
        }
        let code = FrameCode::try_from(bytes[3])?;
        let payload_length = u32::from_be_bytes(bytes[4..8].try_into().expect("fixed slice"));
        let frame_length = HEADER_BYTES
            .checked_add(payload_length as usize)
            .ok_or_else(|| WireError::new("upstream frame length overflow"))?;
        if frame_length > MAX_FRAME_BYTES {
            return Err(WireError::new(format!(
                "upstream frame length {frame_length} exceeds {MAX_FRAME_BYTES}"
            )));
        }
        Ok(Self {
            code,
            payload_length,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Frame {
    pub code: FrameCode,
    pub payload: Vec<u8>,
}

impl Frame {
    pub fn new(code: FrameCode, payload: Vec<u8>) -> Result<Self, WireError> {
        let payload_length = u32::try_from(payload.len())
            .map_err(|_| WireError::new("upstream payload does not fit u32"))?;
        let _ = FrameHeader::decode(
            &FrameHeader {
                code,
                payload_length,
            }
            .encode(),
        )?;
        Ok(Self { code, payload })
    }

    pub fn numeric_id(code: FrameCode, id: u32) -> Self {
        Self {
            code,
            payload: id.to_be_bytes().to_vec(),
        }
    }

    pub fn decode_numeric_id(&self) -> Result<u32, WireError> {
        let bytes: [u8; 4] = self
            .payload
            .as_slice()
            .try_into()
            .map_err(|_| WireError::new("numeric ID payload must contain four bytes"))?;
        Ok(u32::from_be_bytes(bytes))
    }

    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        let payload_length = u32::try_from(self.payload.len())
            .map_err(|_| WireError::new("upstream payload does not fit u32"))?;
        let header = FrameHeader {
            code: self.code,
            payload_length,
        }
        .encode();
        let mut output = Vec::with_capacity(HEADER_BYTES + self.payload.len());
        output.extend_from_slice(&header);
        output.extend_from_slice(&self.payload);
        Ok(output)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForwardedSbFrame {
    pub sb_id: u32,
    pub frame_bytes: Vec<u8>,
}

impl ForwardedSbFrame {
    pub fn new(sb_id: u32, frame_bytes: Vec<u8>) -> Result<Self, WireError> {
        if sb_id == 0 {
            return Err(WireError::new("SB ID zero is reserved"));
        }
        if frame_bytes.is_empty() {
            return Err(WireError::new("forwarded SB frame is empty"));
        }
        Ok(Self { sb_id, frame_bytes })
    }

    pub fn encode(self) -> Vec<u8> {
        let mut output = Vec::with_capacity(4 + self.frame_bytes.len());
        output.extend_from_slice(&self.sb_id.to_be_bytes());
        output.extend_from_slice(&self.frame_bytes);
        output
    }

    pub fn decode(payload: &[u8]) -> Result<Self, WireError> {
        if payload.len() <= 4 {
            return Err(WireError::new("forwarded SB payload is truncated"));
        }
        let sb_id = u32::from_be_bytes(payload[..4].try_into().expect("checked prefix"));
        Self::new(sb_id, payload[4..].to_vec())
    }
}
