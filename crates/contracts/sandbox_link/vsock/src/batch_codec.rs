use sandbox_observation::{
    CpuSnapshot, GuestBootId, GuestPressureSnapshot, GuestResourceSnapshot, MemorySnapshot,
    NormalizedContainerId, Observation, ObservationBatch, OomVictimAttribution,
    OomVictimObservation, ProcessIoCounters, ProcessMarker, PsiAverages, SandboxContainerRuntime,
    WorkloadCgroupCounters, WorkloadCgroupId, WorkloadCgroupResourceSnapshot,
};

use crate::WireError;

const PROCESS_IO_CODE: u8 = 1;
const RESOURCE_CODE: u8 = 2;
const OOM_VICTIM_CODE: u8 = 3;
const PRESSURE_CODE: u8 = 4;
const PRESSURE_BYTES: usize = 48;
const WORKLOAD_CGROUP_CODE: u8 = 5;
const PROCESS_IO_BYTES: usize = 108;
const RESOURCE_BYTES: usize = 74;
const OOM_VICTIM_BYTES: usize = 77;
const WORKLOAD_CGROUP_BYTES: usize = 320;
pub const MAX_ENCODED_OBSERVATION_BYTES: usize = WORKLOAD_CGROUP_BYTES + 3;
pub const OBSERVATION_BATCH_FIXED_BYTES: usize = 10;

#[derive(Clone, Copy, Debug, Default)]
pub struct ObservationBatchCodec;

impl ObservationBatchCodec {
    pub fn encode(&self, batch: &ObservationBatch) -> Result<Vec<u8>, WireError> {
        let count = u16::try_from(batch.observations.len())
            .map_err(|_| WireError::new("observation batch count exceeds u16"))?;
        let estimated = batch
            .observations
            .len()
            .checked_mul(MAX_ENCODED_OBSERVATION_BYTES)
            .and_then(|size| size.checked_add(OBSERVATION_BATCH_FIXED_BYTES))
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
                Observation::OomVictim(value) => {
                    output.push(OOM_VICTIM_CODE);
                    output.extend_from_slice(&(OOM_VICTIM_BYTES as u16).to_be_bytes());
                    self.encode_oom_victim(&mut output, value);
                }
                Observation::GuestPressure(value) => {
                    output.push(PRESSURE_CODE);
                    output.extend_from_slice(&(PRESSURE_BYTES as u16).to_be_bytes());
                    self.encode_pressure(&mut output, value);
                }
                Observation::WorkloadCgroup(value) => {
                    output.push(WORKLOAD_CGROUP_CODE);
                    output.extend_from_slice(&(WORKLOAD_CGROUP_BYTES as u16).to_be_bytes());
                    self.encode_workload_cgroup(&mut output, value);
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
                OOM_VICTIM_CODE if length == OOM_VICTIM_BYTES => {
                    Observation::OomVictim(self.decode_oom_victim(body)?)
                }
                PRESSURE_CODE if length == PRESSURE_BYTES => {
                    Observation::GuestPressure(self.decode_pressure(body)?)
                }
                WORKLOAD_CGROUP_CODE if length == WORKLOAD_CGROUP_BYTES => {
                    Observation::WorkloadCgroup(self.decode_workload_cgroup(body)?)
                }
                PROCESS_IO_CODE | RESOURCE_CODE | OOM_VICTIM_CODE | PRESSURE_CODE
                | WORKLOAD_CGROUP_CODE => {
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

    fn encode_oom_victim(&self, output: &mut Vec<u8>, value: &OomVictimObservation) {
        output.extend_from_slice(value.guest_boot_id.as_bytes());
        output.extend_from_slice(&value.detected_at_ms.to_be_bytes());
        output.extend_from_slice(&value.victim_pid.to_be_bytes());
        output.extend_from_slice(&value.victim_comm);
        output.push(match value.attribution {
            OomVictimAttribution::Unknown => 0,
            OomVictimAttribution::Monitored => 1,
            OomVictimAttribution::Unmonitored => 2,
        });
        let root = value.monitored_root.unwrap_or(ProcessMarker {
            pid: 0,
            start_time_ticks: 0,
            executable_name: [0; 16],
        });
        output.extend_from_slice(&root.pid.to_be_bytes());
        output.extend_from_slice(&root.start_time_ticks.to_be_bytes());
        output.extend_from_slice(&root.executable_name);
        output.extend_from_slice(&[0; 4]);
    }

    fn decode_oom_victim(&self, bytes: &[u8]) -> Result<OomVictimObservation, WireError> {
        let mut cursor = Cursor::new(bytes);
        let guest_boot_id = GuestBootId::new(cursor.array()?);
        let detected_at_ms = cursor.u64()?;
        let victim_pid = cursor.u32()?;
        let victim_comm = cursor.array()?;
        let attribution = match cursor.u8()? {
            0 => OomVictimAttribution::Unknown,
            1 => OomVictimAttribution::Monitored,
            2 => OomVictimAttribution::Unmonitored,
            _ => return Err(WireError::new("invalid OOM victim attribution")),
        };
        let root = ProcessMarker {
            pid: cursor.u32()?,
            start_time_ticks: cursor.u64()?,
            executable_name: cursor.array()?,
        };
        if cursor.array::<4>()? != [0; 4] {
            return Err(WireError::new("non-zero OOM victim reserved bytes"));
        }
        let monitored_root =
            (root.pid != 0 || root.start_time_ticks != 0 || root.executable_name != [0; 16])
                .then_some(root);
        OomVictimObservation {
            guest_boot_id,
            detected_at_ms,
            victim_pid,
            victim_comm,
            attribution,
            monitored_root,
        }
        .validate()
        .map_err(WireError::new)
    }

    fn encode_workload_cgroup(&self, output: &mut Vec<u8>, value: &WorkloadCgroupResourceSnapshot) {
        let slots = workload_cgroup_slots(&value.counters);
        let presence = presence_bitmap(&slots);
        output.push(1);
        output.push(value.runtime.code());
        output.push(if value.container_id.is_some() {
            NormalizedContainerId::HEX_LENGTH as u8
        } else {
            0
        });
        output.push(0);
        output.extend_from_slice(&presence.to_be_bytes());
        output.extend_from_slice(value.guest_boot_id.as_bytes());
        output.extend_from_slice(&value.sampled_at_ms.to_be_bytes());
        output.extend_from_slice(value.workload_id.as_bytes());
        output.extend_from_slice(&value.representative_root.pid.to_be_bytes());
        output.extend_from_slice(&value.monitored_root_count.to_be_bytes());
        output.extend_from_slice(&value.representative_root.start_time_ticks.to_be_bytes());
        output.extend_from_slice(&value.representative_root.executable_name);
        match value.container_id {
            Some(id) => output.extend_from_slice(id.to_lower_hex().as_bytes()),
            None => output.extend_from_slice(&[0; 64]),
        }
        output.extend_from_slice(&value.counters.memory_current_bytes.to_be_bytes());
        for slot in slots {
            output.extend_from_slice(&slot.unwrap_or(0).to_be_bytes());
        }
    }

    fn decode_workload_cgroup(
        &self,
        bytes: &[u8],
    ) -> Result<WorkloadCgroupResourceSnapshot, WireError> {
        let mut cursor = Cursor::new(bytes);
        if cursor.u8()? != 1 {
            return Err(WireError::new("invalid workload cgroup payload version"));
        }
        let runtime = SandboxContainerRuntime::from_code(cursor.u8()?)
            .ok_or_else(|| WireError::new("invalid workload cgroup runtime code"))?;
        let container_id_length = cursor.u8()?;
        if container_id_length != 0
            && container_id_length != NormalizedContainerId::HEX_LENGTH as u8
        {
            return Err(WireError::new(
                "invalid workload cgroup container ID length",
            ));
        }
        if cursor.u8()? != 0 {
            return Err(WireError::new("non-zero workload cgroup reserved byte"));
        }
        let presence = cursor.u32()?;
        if presence >> 19 != 0 {
            return Err(WireError::new(
                "workload cgroup presence bits exceed 19 slots",
            ));
        }
        let guest_boot_id = GuestBootId::new(cursor.array()?);
        let sampled_at_ms = cursor.u64()?;
        let workload_id = WorkloadCgroupId::from_bytes(cursor.array()?);
        let representative_pid = cursor.u32()?;
        let monitored_root_count = cursor.u32()?;
        let representative_root = ProcessMarker {
            pid: representative_pid,
            start_time_ticks: cursor.u64()?,
            executable_name: cursor.array()?,
        };
        let container_bytes: [u8; 64] = cursor.array()?;
        let container_id = if container_bytes.iter().all(|byte| *byte == 0) {
            None
        } else {
            let raw = std::str::from_utf8(&container_bytes)
                .map_err(|_| WireError::new("workload cgroup container ID is not UTF-8"))?;
            Some(NormalizedContainerId::from_lower_hex(raw).map_err(WireError::new)?)
        };
        if (container_id_length == 0) != container_id.is_none() {
            return Err(WireError::new(
                "workload cgroup container ID length does not match payload",
            ));
        }
        let memory_current_bytes = cursor.u64()?;
        let mut slots = [None::<u64>; 19];
        for (index, slot) in slots.iter_mut().enumerate() {
            let value = cursor.u64()?;
            if presence & (1 << index) != 0 {
                *slot = Some(value);
            }
        }
        let counters = WorkloadCgroupCounters {
            memory_current_bytes,
            memory_peak_bytes: slots[0],
            memory_anon_bytes: slots[1],
            memory_file_bytes: slots[2],
            memory_swap_current_bytes: slots[3],
            memory_low: slots[4],
            memory_high: slots[5],
            memory_max: slots[6],
            memory_oom: slots[7],
            memory_oom_kill: slots[8],
            memory_oom_group_kill: slots[9],
            cpu_usage_usec: slots[10],
            cpu_user_usec: slots[11],
            cpu_system_usec: slots[12],
            cpu_nr_throttled: slots[13],
            cpu_throttled_usec: slots[14],
            io_read_bytes: slots[15],
            io_write_bytes: slots[16],
            pids_current: slots[17],
            pids_peak: slots[18],
        };
        let snapshot = WorkloadCgroupResourceSnapshot {
            guest_boot_id,
            sampled_at_ms,
            workload_id,
            representative_root,
            monitored_root_count,
            runtime,
            container_id,
            counters,
        };
        snapshot.validate().map_err(WireError::new)?;
        Ok(snapshot)
    }

    fn encode_pressure(&self, output: &mut Vec<u8>, value: &GuestPressureSnapshot) {
        output.extend_from_slice(value.guest_boot_id.as_bytes());
        output.extend_from_slice(&value.sampled_at_ms.to_be_bytes());
        output.extend_from_slice(&value.memory_some.avg10_millipercent.to_be_bytes());
        output.extend_from_slice(&value.memory_some.avg60_millipercent.to_be_bytes());
        output.extend_from_slice(&value.memory_some.avg300_millipercent.to_be_bytes());
        output.extend_from_slice(&value.memory_full.avg10_millipercent.to_be_bytes());
        output.extend_from_slice(&value.memory_full.avg60_millipercent.to_be_bytes());
        output.extend_from_slice(&value.memory_full.avg300_millipercent.to_be_bytes());
    }

    fn decode_pressure(&self, bytes: &[u8]) -> Result<GuestPressureSnapshot, WireError> {
        let mut cursor = Cursor::new(bytes);
        Ok(GuestPressureSnapshot {
            guest_boot_id: GuestBootId::new(cursor.array()?),
            sampled_at_ms: cursor.u64()?,
            memory_some: PsiAverages {
                avg10_millipercent: cursor.u32()?,
                avg60_millipercent: cursor.u32()?,
                avg300_millipercent: cursor.u32()?,
            },
            memory_full: PsiAverages {
                avg10_millipercent: cursor.u32()?,
                avg60_millipercent: cursor.u32()?,
                avg300_millipercent: cursor.u32()?,
            },
        })
    }
}

fn workload_cgroup_slots(counters: &WorkloadCgroupCounters) -> [Option<u64>; 19] {
    [
        counters.memory_peak_bytes,
        counters.memory_anon_bytes,
        counters.memory_file_bytes,
        counters.memory_swap_current_bytes,
        counters.memory_low,
        counters.memory_high,
        counters.memory_max,
        counters.memory_oom,
        counters.memory_oom_kill,
        counters.memory_oom_group_kill,
        counters.cpu_usage_usec,
        counters.cpu_user_usec,
        counters.cpu_system_usec,
        counters.cpu_nr_throttled,
        counters.cpu_throttled_usec,
        counters.io_read_bytes,
        counters.io_write_bytes,
        counters.pids_current,
        counters.pids_peak,
    ]
}

fn presence_bitmap(slots: &[Option<u64>; 19]) -> u32 {
    let mut bitmap = 0_u32;
    for (index, slot) in slots.iter().enumerate() {
        if slot.is_some() {
            bitmap |= 1 << index;
        }
    }
    bitmap
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

#[cfg(test)]
mod tests {
    use super::*;
    use sandbox_observation::GuestBootId;

    const ID: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn full_snapshot() -> WorkloadCgroupResourceSnapshot {
        WorkloadCgroupResourceSnapshot {
            guest_boot_id: GuestBootId::new([0x11; 16]),
            sampled_at_ms: 0x0102_0304_0506_0708,
            workload_id: WorkloadCgroupId::from_bytes([0x22; 32]),
            representative_root: ProcessMarker {
                pid: 1234,
                start_time_ticks: 5678,
                executable_name: *b"root-process!!!!",
            },
            monitored_root_count: 3,
            runtime: SandboxContainerRuntime::Docker,
            container_id: Some(NormalizedContainerId::from_lower_hex(ID).unwrap()),
            counters: WorkloadCgroupCounters {
                memory_current_bytes: 1,
                memory_peak_bytes: Some(2),
                memory_anon_bytes: Some(3),
                memory_file_bytes: Some(4),
                memory_swap_current_bytes: Some(5),
                memory_low: Some(6),
                memory_high: Some(7),
                memory_max: Some(8),
                memory_oom: Some(9),
                memory_oom_kill: Some(10),
                memory_oom_group_kill: Some(11),
                cpu_usage_usec: Some(12),
                cpu_user_usec: Some(13),
                cpu_system_usec: Some(14),
                cpu_nr_throttled: Some(15),
                cpu_throttled_usec: Some(16),
                io_read_bytes: Some(17),
                io_write_bytes: Some(18),
                pids_current: Some(19),
                pids_peak: Some(20),
            },
        }
    }

    fn minimal_snapshot() -> WorkloadCgroupResourceSnapshot {
        WorkloadCgroupResourceSnapshot {
            guest_boot_id: GuestBootId::new([0x33; 16]),
            sampled_at_ms: 7,
            workload_id: WorkloadCgroupId::from_bytes([0x44; 32]),
            representative_root: ProcessMarker {
                pid: 99,
                start_time_ticks: 100,
                executable_name: [0; 16],
            },
            monitored_root_count: 1,
            runtime: SandboxContainerRuntime::Unknown,
            container_id: None,
            counters: WorkloadCgroupCounters {
                memory_current_bytes: 42,
                ..WorkloadCgroupCounters::default()
            },
        }
    }

    fn codec() -> ObservationBatchCodec {
        ObservationBatchCodec
    }

    #[test]
    fn workload_cgroup_body_is_exactly_320_bytes_and_round_trips() {
        let mut body = Vec::new();
        codec().encode_workload_cgroup(&mut body, &full_snapshot());
        assert_eq!(body.len(), 320);
        assert_eq!(body[0], 1); // payload version
        assert_eq!(body[1], SandboxContainerRuntime::Docker.code());
        assert_eq!(body[2], NormalizedContainerId::HEX_LENGTH as u8);
        assert_eq!(body[3], 0); // reserved

        let decoded = codec().decode_workload_cgroup(&body).unwrap();
        assert_eq!(decoded, full_snapshot());
    }

    #[test]
    fn workload_cgroup_round_trips_absent_fields_and_container() {
        let mut body = Vec::new();
        codec().encode_workload_cgroup(&mut body, &minimal_snapshot());
        assert_eq!(body.len(), 320);
        assert_eq!(body[2], 0); // absent container ID
        let decoded = codec().decode_workload_cgroup(&body).unwrap();
        assert_eq!(decoded, minimal_snapshot());
        assert!(decoded.counters.memory_peak_bytes.is_none());
        assert!(decoded.counters.cpu_usage_usec.is_none());
        assert_eq!(decoded.container_id, None);
    }

    #[test]
    fn workload_cgroup_round_trips_through_batch_codec() {
        let batch = ObservationBatch::new(
            9,
            vec![
                Observation::WorkloadCgroup(full_snapshot()),
                Observation::WorkloadCgroup(minimal_snapshot()),
            ],
        );
        let encoded = codec().encode(&batch).unwrap();
        let decoded = codec().decode(&encoded).unwrap();
        assert_eq!(decoded.sequence, 9);
        assert_eq!(decoded.observations.len(), 2);
        assert_eq!(
            decoded.observations[0],
            Observation::WorkloadCgroup(full_snapshot())
        );
        assert_eq!(
            decoded.observations[1],
            Observation::WorkloadCgroup(minimal_snapshot())
        );
    }

    #[test]
    fn workload_cgroup_rejects_invalid_version_runtime_and_zero_roots() {
        let mut body = Vec::new();
        codec().encode_workload_cgroup(&mut body, &minimal_snapshot());

        let mut bad_version = body.clone();
        bad_version[0] = 2;
        assert!(codec().decode_workload_cgroup(&bad_version).is_err());

        let mut bad_runtime = body.clone();
        bad_runtime[1] = 6;
        assert!(codec().decode_workload_cgroup(&bad_runtime).is_err());

        let mut bad_reserved = body.clone();
        bad_reserved[3] = 1;
        assert!(codec().decode_workload_cgroup(&bad_reserved).is_err());

        let mut zero_roots = body.clone();
        zero_roots[68..72].copy_from_slice(&0_u32.to_be_bytes());
        assert!(codec().decode_workload_cgroup(&zero_roots).is_err());

        let mut high_presence = body.clone();
        high_presence[4..8].copy_from_slice(&0x8000_0000_u32.to_be_bytes());
        assert!(codec().decode_workload_cgroup(&high_presence).is_err());
    }

    #[test]
    fn workload_cgroup_rejects_truncated_and_trailing_bytes() {
        let mut body = Vec::new();
        codec().encode_workload_cgroup(&mut body, &minimal_snapshot());

        assert!(codec().decode_workload_cgroup(&body[..319]).is_err());

        let mut trailing = body.clone();
        trailing.push(0);
        // decode_workload_cgroup does not itself check trailing bytes; the batch
        // codec does. Verify batch-level trailing rejection instead.
        let batch = ObservationBatch::new(1, vec![Observation::WorkloadCgroup(minimal_snapshot())]);
        let mut encoded = codec().encode(&batch).unwrap();
        encoded.push(0);
        assert!(codec().decode(&encoded).is_err());
    }
}

#[cfg(test)]
mod pressure_tests {
    use super::*;

    fn pressure() -> GuestPressureSnapshot {
        GuestPressureSnapshot {
            guest_boot_id: GuestBootId::new([9; 16]),
            sampled_at_ms: 1_234,
            memory_some: PsiAverages {
                avg10_millipercent: 2_350,
                avg60_millipercent: 1_100,
                avg300_millipercent: 500,
            },
            memory_full: PsiAverages {
                avg10_millipercent: 100_000,
                avg60_millipercent: 0,
                avg300_millipercent: 42_000,
            },
        }
    }

    #[test]
    fn pressure_observation_round_trips() {
        let codec = ObservationBatchCodec;
        let batch = ObservationBatch::new(7, vec![Observation::GuestPressure(pressure())]);
        let encoded = codec.encode(&batch).expect("encode pressure");
        assert_eq!(encoded.len(), 10 + 3 + PRESSURE_BYTES);
        assert_eq!(encoded[10], PRESSURE_CODE);
        assert_eq!(codec.decode(&encoded).expect("decode pressure"), batch);
    }

    #[test]
    fn pressure_rejects_wrong_body_length() {
        let codec = ObservationBatchCodec;
        let mut encoded = codec
            .encode(&ObservationBatch::new(
                1,
                vec![Observation::GuestPressure(pressure())],
            ))
            .expect("encode pressure");
        // Corrupt the body length so it no longer matches PRESSURE_BYTES.
        encoded[11] = 0;
        encoded[12] = (PRESSURE_BYTES + 1) as u8;
        assert!(codec.decode(&encoded).is_err());
    }
}
