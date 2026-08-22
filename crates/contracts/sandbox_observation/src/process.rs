use std::fmt;

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub struct GuestBootId([u8; 16]);

impl GuestBootId {
    pub const ZERO: Self = Self([0; 16]);

    pub const fn new(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

impl fmt::Debug for GuestBootId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("GuestBootId").field(&self.0).finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProcessMarker {
    pub pid: u32,
    pub start_time_ticks: u64,
    pub executable_name: [u8; 16],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProcessIoCounters {
    pub guest_boot_id: GuestBootId,
    pub process: ProcessMarker,
    pub sample_started_ms: u64,
    pub sample_ended_ms: u64,
    pub read_operations: u64,
    pub read_bytes: u64,
    pub write_operations: u64,
    pub write_bytes: u64,
    pub failed_read_operations: u64,
    pub failed_write_operations: u64,
}
