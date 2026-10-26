use crate::WireError;

const MAGIC: u16 = 0xac72;
const VERSION: u8 = 1;
pub const MAX_FRAME_BYTES: usize = 272 * 1024;
pub const HEADER_BYTES: usize = 8;
const WORKLOAD_CGROUP_CAPABILITY: u32 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GatewayWelcome {
    pub gateway_id: u32,
    pub workload_cgroup_observations: bool,
}

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

    pub fn gateway_hello(workload_cgroup_observations: bool) -> Self {
        let payload = if workload_cgroup_observations {
            WORKLOAD_CGROUP_CAPABILITY.to_be_bytes().to_vec()
        } else {
            Vec::new()
        };
        Self {
            code: FrameCode::GatewayHello,
            payload,
        }
    }

    pub fn decode_gateway_hello(&self) -> Result<bool, WireError> {
        if self.code != FrameCode::GatewayHello {
            return Err(WireError::new("frame is not a gateway hello"));
        }
        if self.payload.is_empty() {
            return Ok(false);
        }
        let flags = self.decode_numeric_id()?;
        if flags & !WORKLOAD_CGROUP_CAPABILITY != 0 {
            return Err(WireError::new(
                "gateway hello contains unknown capabilities",
            ));
        }
        Ok(flags & WORKLOAD_CGROUP_CAPABILITY != 0)
    }

    pub fn gateway_welcome(
        gateway_id: u32,
        workload_cgroup_observations: bool,
    ) -> Result<Self, WireError> {
        if gateway_id == 0 {
            return Err(WireError::new("gateway ID must be non-zero"));
        }
        let mut payload = gateway_id.to_be_bytes().to_vec();
        if workload_cgroup_observations {
            payload.extend_from_slice(&WORKLOAD_CGROUP_CAPABILITY.to_be_bytes());
        }
        Self::new(FrameCode::GatewayWelcome, payload)
    }

    pub fn decode_gateway_welcome(&self) -> Result<GatewayWelcome, WireError> {
        if self.code != FrameCode::GatewayWelcome {
            return Err(WireError::new("frame is not a gateway welcome"));
        }
        let (id_bytes, flags) = match self.payload.len() {
            4 => (&self.payload[..4], 0),
            8 => {
                let flags =
                    u32::from_be_bytes(self.payload[4..8].try_into().expect("checked length"));
                (&self.payload[..4], flags)
            }
            _ => return Err(WireError::new("invalid gateway welcome payload length")),
        };
        if flags & !WORKLOAD_CGROUP_CAPABILITY != 0 {
            return Err(WireError::new(
                "gateway welcome contains unknown capabilities",
            ));
        }
        let gateway_id = u32::from_be_bytes(id_bytes.try_into().expect("checked length"));
        if gateway_id == 0 {
            return Err(WireError::new("gateway welcome contains reserved ID zero"));
        }
        Ok(GatewayWelcome {
            gateway_id,
            workload_cgroup_observations: flags & WORKLOAD_CGROUP_CAPABILITY != 0,
        })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gateway_capability_handshake_is_legacy_compatible() {
        let legacy_hello = Frame::gateway_hello(false);
        assert!(legacy_hello.payload.is_empty());
        assert!(!legacy_hello.decode_gateway_hello().unwrap());
        let legacy_welcome = Frame::gateway_welcome(3, false).unwrap();
        assert_eq!(legacy_welcome.payload.len(), 4);
        assert_eq!(
            legacy_welcome.decode_gateway_welcome().unwrap(),
            GatewayWelcome {
                gateway_id: 3,
                workload_cgroup_observations: false,
            }
        );

        let hello = Frame::gateway_hello(true);
        assert!(hello.decode_gateway_hello().unwrap());
        let welcome = Frame::gateway_welcome(7, true).unwrap();
        assert_eq!(welcome.payload.len(), 8);
        assert_eq!(
            welcome.decode_gateway_welcome().unwrap(),
            GatewayWelcome {
                gateway_id: 7,
                workload_cgroup_observations: true,
            }
        );
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
