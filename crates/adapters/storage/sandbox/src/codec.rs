use sandbox_evidence_store::sandbox_observation::{
    CpuSnapshot, GuestBootId, GuestResourceSnapshot, MemorySnapshot, Observation,
    ProcessIoCounters, ProcessMarker,
};

const PROCESS_IO_KIND: u8 = 1;
const GUEST_RESOURCE_KIND: u8 = 2;

pub(super) struct ObservationCodec;

impl ObservationCodec {
    pub(super) fn encode(observation: &Observation) -> (u8, Vec<u8>) {
        let mut bytes = Vec::with_capacity(104);
        match observation {
            Observation::ProcessIo(value) => {
                bytes.extend_from_slice(value.guest_boot_id.as_bytes());
                bytes.extend_from_slice(&value.process.pid.to_be_bytes());
                bytes.extend_from_slice(&value.process.start_time_ticks.to_be_bytes());
                bytes.extend_from_slice(&value.process.executable_name);
                for field in [
                    value.sample_started_ms,
                    value.sample_ended_ms,
                    value.read_operations,
                    value.read_bytes,
                    value.write_operations,
                    value.write_bytes,
                    value.failed_read_operations,
                    value.failed_write_operations,
                ] {
                    bytes.extend_from_slice(&field.to_be_bytes());
                }
                (PROCESS_IO_KIND, bytes)
            }
            Observation::GuestResource(value) => {
                bytes.extend_from_slice(value.guest_boot_id.as_bytes());
                for field in [
                    value.sampled_at_ms,
                    value.cpu.total_ticks,
                    value.cpu.idle_ticks,
                ] {
                    bytes.extend_from_slice(&field.to_be_bytes());
                }
                bytes.extend_from_slice(&value.cpu.logical_cpu_count.to_be_bytes());
                for field in [
                    value.memory.total_bytes,
                    value.memory.available_bytes,
                    value.memory.used_bytes,
                    value.memory.oom_kill_count,
                ] {
                    bytes.extend_from_slice(&field.to_be_bytes());
                }
                (GUEST_RESOURCE_KIND, bytes)
            }
        }
    }

    pub(super) fn decode(kind: u8, bytes: &[u8]) -> Result<Observation, String> {
        let mut cursor = PayloadCursor::new(bytes);
        let observation = match kind {
            PROCESS_IO_KIND => Observation::ProcessIo(ProcessIoCounters {
                guest_boot_id: GuestBootId::new(cursor.array()?),
                process: ProcessMarker {
                    pid: cursor.u32()?,
                    start_time_ticks: cursor.u64()?,
                    executable_name: cursor.array()?,
                },
                sample_started_ms: cursor.u64()?,
                sample_ended_ms: cursor.u64()?,
                read_operations: cursor.u64()?,
                read_bytes: cursor.u64()?,
                write_operations: cursor.u64()?,
                write_bytes: cursor.u64()?,
                failed_read_operations: cursor.u64()?,
                failed_write_operations: cursor.u64()?,
            }),
            GUEST_RESOURCE_KIND => Observation::GuestResource(GuestResourceSnapshot {
                guest_boot_id: GuestBootId::new(cursor.array()?),
                sampled_at_ms: cursor.u64()?,
                cpu: CpuSnapshot {
                    total_ticks: cursor.u64()?,
                    idle_ticks: cursor.u64()?,
                    logical_cpu_count: cursor.u16()?,
                },
                memory: MemorySnapshot {
                    total_bytes: cursor.u64()?,
                    available_bytes: cursor.u64()?,
                    used_bytes: cursor.u64()?,
                    oom_kill_count: cursor.u64()?,
                },
            }),
            _ => return Err(format!("unknown sandbox evidence observation kind {kind}")),
        };
        if cursor.remaining() != 0 {
            return Err("sandbox evidence payload has trailing bytes".to_string());
        }
        Ok(observation)
    }
}

struct PayloadCursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> PayloadCursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], String> {
        let end = self
            .offset
            .checked_add(N)
            .ok_or_else(|| "sandbox evidence payload offset overflow".to_string())?;
        let slice = self
            .bytes
            .get(self.offset..end)
            .ok_or_else(|| "sandbox evidence payload is truncated".to_string())?;
        self.offset = end;
        slice
            .try_into()
            .map_err(|_| "sandbox evidence payload width mismatch".to_string())
    }

    fn u16(&mut self) -> Result<u16, String> {
        self.array().map(u16::from_be_bytes)
    }

    fn u32(&mut self) -> Result<u32, String> {
        self.array().map(u32::from_be_bytes)
    }

    fn u64(&mut self) -> Result<u64, String> {
        self.array().map(u64::from_be_bytes)
    }

    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.offset)
    }
}
