use sandbox_observation::{
    CpuSnapshot, GuestBootId, GuestResourceSnapshot, MemorySnapshot, Observation, ObservationBatch,
    ProcessIoCounters, ProcessMarker,
};

use crate::WireError;

const PROCESS_IO_CODE: u8 = 1;
const RESOURCE_CODE: u8 = 2;
const PROCESS_IO_BYTES: usize = 108;
const RESOURCE_BYTES: usize = 74;

#[derive(Clone, Copy, Debug, Default)]
pub struct ObservationBatchCodec;

impl ObservationBatchCodec {
    pub fn encode(&self, batch: &ObservationBatch) -> Result<Vec<u8>, WireError> {
        let count = u16::try_from(batch.observations.len())
            .map_err(|_| WireError::new("observation batch count exceeds u16"))?;
        let estimated = batch
            .observations
            .len()
            .checked_mul(PROCESS_IO_BYTES + 3)
            .and_then(|size| size.checked_add(10))
            .ok_or_else(|| WireError::new("observation batch size overflow"))?;
        let mut output = Vec::with_capacity(estimated);
        output.extend_from_slice(&batch.sequence.to_be_bytes());
        output.extend_from_slice(&count.to_be_bytes());
        for observation in &batch.observations {
            match observation {
                Observation::ProcessIo(value) => {
                    output.push(PROCESS_IO_CODE);
                    output.extend_from_slice(&(PROCESS_IO_BYTES as u16).to_be_bytes());
                    self.encode_process_io(&mut output, value);
                }
                Observation::GuestResource(value) => {
                    output.push(RESOURCE_CODE);
                    output.extend_from_slice(&(RESOURCE_BYTES as u16).to_be_bytes());
                    self.encode_resource(&mut output, value);
                }
            }
        }
        Ok(output)
    }

    pub fn decode(&self, bytes: &[u8]) -> Result<ObservationBatch, WireError> {
        let mut cursor = Cursor::new(bytes);
        let sequence = cursor.u64()?;
        let count = cursor.u16()? as usize;
        let mut observations = Vec::with_capacity(count);
        for _ in 0..count {
            let code = cursor.u8()?;
            let length = cursor.u16()? as usize;
            let body = cursor.take(length)?;
            let observation = match code {
                PROCESS_IO_CODE if length == PROCESS_IO_BYTES => {
                    Observation::ProcessIo(self.decode_process_io(body)?)
                }
                RESOURCE_CODE if length == RESOURCE_BYTES => {
                    Observation::GuestResource(self.decode_resource(body)?)
                }
                PROCESS_IO_CODE | RESOURCE_CODE => {
                    return Err(WireError::new(format!(
                        "invalid observation body length {length} for code {code}"
                    )));
                }
                other => {
                    return Err(WireError::new(format!("unknown observation code {other}")));
                }
            };
            observations.push(observation);
        }
        if cursor.remaining() != 0 {
            return Err(WireError::new("trailing bytes in observation batch"));
        }
        Ok(ObservationBatch::new(sequence, observations))
    }

    fn encode_process_io(&self, output: &mut Vec<u8>, value: &ProcessIoCounters) {
        output.extend_from_slice(value.guest_boot_id.as_bytes());
        output.extend_from_slice(&value.process.pid.to_be_bytes());
        output.extend_from_slice(&value.process.start_time_ticks.to_be_bytes());
        output.extend_from_slice(&value.process.executable_name);
        output.extend_from_slice(&value.sample_started_ms.to_be_bytes());
        output.extend_from_slice(&value.sample_ended_ms.to_be_bytes());
        output.extend_from_slice(&value.read_operations.to_be_bytes());
        output.extend_from_slice(&value.read_bytes.to_be_bytes());
        output.extend_from_slice(&value.write_operations.to_be_bytes());
        output.extend_from_slice(&value.write_bytes.to_be_bytes());
        output.extend_from_slice(&value.failed_read_operations.to_be_bytes());
        output.extend_from_slice(&value.failed_write_operations.to_be_bytes());
    }

    fn decode_process_io(&self, bytes: &[u8]) -> Result<ProcessIoCounters, WireError> {
        let mut cursor = Cursor::new(bytes);
        Ok(ProcessIoCounters {
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
        })
    }

    fn encode_resource(&self, output: &mut Vec<u8>, value: &GuestResourceSnapshot) {
        output.extend_from_slice(value.guest_boot_id.as_bytes());
        output.extend_from_slice(&value.sampled_at_ms.to_be_bytes());
        output.extend_from_slice(&value.cpu.total_ticks.to_be_bytes());
        output.extend_from_slice(&value.cpu.idle_ticks.to_be_bytes());
        output.extend_from_slice(&value.cpu.logical_cpu_count.to_be_bytes());
        output.extend_from_slice(&value.memory.total_bytes.to_be_bytes());
        output.extend_from_slice(&value.memory.available_bytes.to_be_bytes());
        output.extend_from_slice(&value.memory.used_bytes.to_be_bytes());
        output.extend_from_slice(&value.memory.oom_kill_count.to_be_bytes());
    }

    fn decode_resource(&self, bytes: &[u8]) -> Result<GuestResourceSnapshot, WireError> {
        let mut cursor = Cursor::new(bytes);
        Ok(GuestResourceSnapshot {
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
        })
    }
}

struct Cursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.offset)
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], WireError> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or_else(|| WireError::new("wire cursor overflow"))?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or_else(|| WireError::new("truncated wire payload"))?;
        self.offset = end;
        Ok(value)
    }

    fn u8(&mut self) -> Result<u8, WireError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, WireError> {
        Ok(u16::from_be_bytes(self.array()?))
    }

    fn u32(&mut self) -> Result<u32, WireError> {
        Ok(u32::from_be_bytes(self.array()?))
    }

    fn u64(&mut self) -> Result<u64, WireError> {
        Ok(u64::from_be_bytes(self.array()?))
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], WireError> {
        self.take(N)?
            .try_into()
            .map_err(|_| WireError::new("invalid fixed-width wire field"))
    }
}
