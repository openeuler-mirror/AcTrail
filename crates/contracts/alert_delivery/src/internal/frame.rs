use crate::DeliveryCodecError;

const ATAP_MAGIC: [u8; 4] = *b"ATAP";
const ATAP_VERSION: u8 = 2;
pub const ATAP_HEADER_BYTES: usize = 12;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum AtapMessageCode {
    ProducerHello = 0x01,
    ProducerWelcome = 0x02,
    ProducerReject = 0x03,
    ForwardAlert = 0x10,
    Heartbeat = 0x20,
    HeartbeatAck = 0x21,
}

impl TryFrom<u8> for AtapMessageCode {
    type Error = DeliveryCodecError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x01 => Ok(Self::ProducerHello),
            0x02 => Ok(Self::ProducerWelcome),
            0x03 => Ok(Self::ProducerReject),
            0x10 => Ok(Self::ForwardAlert),
            0x20 => Ok(Self::Heartbeat),
            0x21 => Ok(Self::HeartbeatAck),
            other => Err(DeliveryCodecError::new(
                "atap_header",
                format!("unknown message code {other:#04x}"),
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AtapHeader {
    pub(crate) code: AtapMessageCode,
    pub(crate) payload_length: u32,
}

impl AtapHeader {
    pub(crate) fn encode(self) -> [u8; ATAP_HEADER_BYTES] {
        let mut bytes = [0_u8; ATAP_HEADER_BYTES];
        bytes[..4].copy_from_slice(&ATAP_MAGIC);
        bytes[4] = ATAP_VERSION;
        bytes[5] = self.code as u8;
        bytes[8..12].copy_from_slice(&self.payload_length.to_be_bytes());
        bytes
    }

    pub(crate) fn decode(bytes: &[u8; ATAP_HEADER_BYTES]) -> Result<Self, DeliveryCodecError> {
        if bytes[..4] != ATAP_MAGIC {
            return Err(DeliveryCodecError::new("atap_header", "invalid ATAP magic"));
        }
        if bytes[4] != ATAP_VERSION {
            return Err(DeliveryCodecError::new(
                "atap_header",
                format!("unsupported ATAP version {}", bytes[4]),
            ));
        }
        if bytes[6] != 0 || bytes[7] != 0 {
            return Err(DeliveryCodecError::new(
                "atap_header",
                "reserved header bytes must be zero",
            ));
        }
        Ok(Self {
            code: AtapMessageCode::try_from(bytes[5])?,
            payload_length: u32::from_be_bytes(
                bytes[8..12].try_into().expect("fixed header field"),
            ),
        })
    }
}
