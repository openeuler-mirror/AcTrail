#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SandboxSource {
    gateway_id: u32,
    sb_id: u32,
}

impl SandboxSource {
    pub const fn new(gateway_id: u32, sb_id: u32) -> Result<Self, SandboxSourceError> {
        if gateway_id == 0 {
            return Err(SandboxSourceError::ZeroGatewayId);
        }
        if sb_id == 0 {
            return Err(SandboxSourceError::ZeroSbId);
        }
        Ok(Self { gateway_id, sb_id })
    }

    pub const fn gateway_id(self) -> u32 {
        self.gateway_id
    }

    pub const fn sb_id(self) -> u32 {
        self.sb_id
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SandboxSourceError {
    ZeroGatewayId,
    ZeroSbId,
}
