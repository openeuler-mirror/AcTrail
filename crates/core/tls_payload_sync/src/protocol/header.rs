use crate::{SyncError, SyncResult};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum FrameKind {
    Payload = 1,
    Decision = 2,
    Summary = 3,
    PlanLookup = 4,
    PlanFound = 5,
    PlanUnavailable = 6,
}

impl FrameKind {
    fn decode(value: u8) -> SyncResult<Self> {
        match value {
            1 => Ok(Self::Payload),
            2 => Ok(Self::Decision),
            3 => Ok(Self::Summary),
            4 => Ok(Self::PlanLookup),
            5 => Ok(Self::PlanFound),
            6 => Ok(Self::PlanUnavailable),
            _ => Err(SyncError::new("unknown TLS frame kind")),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct FrameHeader {
    kind: FrameKind,
    body_len: u32,
}

impl FrameHeader {
    pub const LEN: usize = payload_capability::TLS_SYNC_FRAME_HEADER_BYTES;
    const MAGIC: [u8; 2] = *b"AT";
    const VERSION: u8 = 1;

    pub fn frame_len(self) -> usize {
        Self::LEN + self.body_len()
    }
    pub fn body_len(self) -> usize {
        self.body_len as usize
    }
    pub fn kind(self) -> FrameKind {
        self.kind
    }

    pub(super) fn new(
        kind: FrameKind,
        body_len: usize,
        max_frame_bytes: usize,
    ) -> SyncResult<Self> {
        let frame_len = body_len
            .checked_add(Self::LEN)
            .ok_or_else(|| SyncError::new("TLS frame length overflow"))?;
        if frame_len > max_frame_bytes {
            return Err(SyncError::new("TLS frame exceeds configured maximum"));
        }
        let body_len = u32::try_from(body_len)
            .map_err(|_| SyncError::new("TLS frame body exceeds wire length range"))?;
        Ok(Self { kind, body_len })
    }

    pub(super) fn parse(bytes: &[u8], max_frame_bytes: usize) -> SyncResult<Option<Self>> {
        if bytes.len() < Self::LEN {
            return Ok(None);
        }
        if bytes[..2] != Self::MAGIC {
            return Err(SyncError::new("invalid TLS frame magic"));
        }
        if bytes[2] != Self::VERSION {
            return Err(SyncError::new("unknown TLS frame version"));
        }
        let kind = FrameKind::decode(bytes[3])?;
        let body_len = u32::from_le_bytes(bytes[4..8].try_into().expect("fixed header length"));
        Self::new(kind, body_len as usize, max_frame_bytes).map(Some)
    }

    pub(super) fn encode(self) -> [u8; Self::LEN] {
        let mut bytes = [0; Self::LEN];
        bytes[..2].copy_from_slice(&Self::MAGIC);
        bytes[2] = Self::VERSION;
        bytes[3] = self.kind as u8;
        bytes[4..].copy_from_slice(&self.body_len.to_le_bytes());
        bytes
    }
}
