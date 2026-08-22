use crate::WireError;

const MAGIC: u16 = 0xac71;
const VERSION: u8 = 1;
pub const MAX_FRAME_BYTES: usize = 256 * 1024;
pub const HEADER_BYTES: usize = 8;

#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrameCode {
    SbHello = 1,
    SbWelcome = 2,
    Heartbeat = 3,
    ObservationBatch = 16,
}

impl TryFrom<u8> for FrameCode {
    type Error = WireError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::SbHello),
            2 => Ok(Self::SbWelcome),
            3 => Ok(Self::Heartbeat),
            16 => Ok(Self::ObservationBatch),
            other => Err(WireError::new(format!("unknown SB frame code {other}"))),
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
        let magic = u16::from_be_bytes([bytes[0], bytes[1]]);
        if magic != MAGIC {
            return Err(WireError::new("invalid SB frame magic"));
        }
        if bytes[2] != VERSION {
            return Err(WireError::new(format!(
                "unsupported SB protocol version {}",
                bytes[2]
            )));
        }
        let code = FrameCode::try_from(bytes[3])?;
        let payload_length = u32::from_be_bytes(bytes[4..8].try_into().expect("fixed slice"));
        let frame_length = HEADER_BYTES
            .checked_add(payload_length as usize)
            .ok_or_else(|| WireError::new("SB frame length overflow"))?;
        if frame_length > MAX_FRAME_BYTES {
            return Err(WireError::new(format!(
                "SB frame length {frame_length} exceeds {MAX_FRAME_BYTES}"
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
            .map_err(|_| WireError::new("SB payload does not fit u32"))?;
        let header = FrameHeader {
            code,
            payload_length,
        };
        let _ = FrameHeader::decode(&header.encode())?;
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
            .map_err(|_| WireError::new("SB payload does not fit u32"))?;
        let header = FrameHeader {
            code: self.code,
            payload_length,
        }
        .encode();
        let mut bytes = Vec::with_capacity(HEADER_BYTES + self.payload.len());
        bytes.extend_from_slice(&header);
        bytes.extend_from_slice(&self.payload);
        Ok(bytes)
    }
}
