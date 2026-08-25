use sandbox_alert_store::SandboxAlertKind;
use sandbox_observation::{GuestBootId, OomVictimAttribution, ProcessMarker};

const HIGH_CPU: u8 = 1;
const OOM_KILLED: u8 = 2;
const OOM_RISK: u8 = 3;
const HIGH_READ: u8 = 4;
const HIGH_WRITE: u8 = 5;

pub(super) struct AlertCodec;

impl AlertCodec {
    pub(super) fn encode(kind: SandboxAlertKind) -> (u8, Vec<u8>) {
        match kind {
            SandboxAlertKind::HighCpu {
                guest_boot_id,
                usage_basis_points,
                threshold_basis_points,
                ..
            } => {
                let mut payload = Vec::with_capacity(20);
                payload.extend_from_slice(guest_boot_id.as_bytes());
                payload.extend_from_slice(&usage_basis_points.to_be_bytes());
                payload.extend_from_slice(&threshold_basis_points.to_be_bytes());
                (HIGH_CPU, payload)
            }
            SandboxAlertKind::OomKilled {
                guest_boot_id,
                victim_pid,
                victim_comm,
                attribution,
                monitored_root,
                ..
            } => {
                let mut payload = Vec::with_capacity(68);
                payload.extend_from_slice(guest_boot_id.as_bytes());
                payload.extend_from_slice(&victim_pid.to_be_bytes());
                payload.extend_from_slice(&victim_comm);
                payload.push(encode_attribution(attribution));
                payload.push(u8::from(monitored_root.is_some()));
                payload.extend_from_slice(&[0; 2]);
                encode_optional_process(&mut payload, monitored_root);
                (OOM_KILLED, payload)
            }
            SandboxAlertKind::OomRisk {
                guest_boot_id,
                available_bytes,
                threshold_bytes,
                ..
            } => {
                let mut payload = Vec::with_capacity(32);
                payload.extend_from_slice(guest_boot_id.as_bytes());
                append_u64(&mut payload, available_bytes);
                append_u64(&mut payload, threshold_bytes);
                (OOM_RISK, payload)
            }
            SandboxAlertKind::HighRead {
                guest_boot_id,
                process,
                sample_started_ms,
                bytes,
                threshold_bytes,
                ..
            } => (
                HIGH_READ,
                encode_process_alert(
                    guest_boot_id,
                    process,
                    sample_started_ms,
                    bytes,
                    threshold_bytes,
                ),
            ),
            SandboxAlertKind::HighWrite {
                guest_boot_id,
                process,
                sample_started_ms,
                bytes,
                threshold_bytes,
                ..
            } => (
                HIGH_WRITE,
                encode_process_alert(
                    guest_boot_id,
                    process,
                    sample_started_ms,
                    bytes,
                    threshold_bytes,
                ),
            ),
        }
    }

    pub(super) fn decode(
        kind: u8,
        detected_at_ms: u64,
        payload: &[u8],
    ) -> Result<SandboxAlertKind, String> {
        let mut decoder = Decoder::new(payload);
        let guest_boot_id = GuestBootId::new(decoder.array::<16>()?);
        let alert = match kind {
            HIGH_CPU => SandboxAlertKind::HighCpu {
                guest_boot_id,
                sampled_at_ms: detected_at_ms,
                usage_basis_points: decoder.u16()?,
                threshold_basis_points: decoder.u16()?,
            },
            OOM_KILLED => SandboxAlertKind::OomKilled {
                guest_boot_id,
                detected_at_ms,
                victim_pid: decoder.u32()?,
                victim_comm: decoder.array::<16>()?,
                attribution: decode_attribution(decoder.u8()?)?,
                monitored_root: decode_optional_process(&mut decoder)?,
            },
            OOM_RISK => SandboxAlertKind::OomRisk {
                guest_boot_id,
                sampled_at_ms: detected_at_ms,
                available_bytes: decoder.u64()?,
                threshold_bytes: decoder.u64()?,
            },
            HIGH_READ | HIGH_WRITE => {
                let process = ProcessMarker {
                    pid: decoder.u32()?,
                    start_time_ticks: decoder.u64()?,
                    executable_name: decoder.array::<16>()?,
                };
                let sample_started_ms = decoder.u64()?;
                let bytes = decoder.u64()?;
                let threshold_bytes = decoder.u64()?;
                if kind == HIGH_READ {
                    SandboxAlertKind::HighRead {
                        guest_boot_id,
                        process,
                        sample_started_ms,
                        sample_ended_ms: detected_at_ms,
                        bytes,
                        threshold_bytes,
                    }
                } else {
                    SandboxAlertKind::HighWrite {
                        guest_boot_id,
                        process,
                        sample_started_ms,
                        sample_ended_ms: detected_at_ms,
                        bytes,
                        threshold_bytes,
                    }
                }
            }
            _ => return Err(format!("unknown sandbox alert kind {kind}")),
        };
        decoder.finish()?;
        Ok(alert)
    }
}

fn encode_attribution(attribution: OomVictimAttribution) -> u8 {
    match attribution {
        OomVictimAttribution::Unknown => 0,
        OomVictimAttribution::Monitored => 1,
        OomVictimAttribution::Unmonitored => 2,
    }
}

fn decode_attribution(raw: u8) -> Result<OomVictimAttribution, String> {
    match raw {
        0 => Ok(OomVictimAttribution::Unknown),
        1 => Ok(OomVictimAttribution::Monitored),
        2 => Ok(OomVictimAttribution::Unmonitored),
        _ => Err("invalid stored OOM victim attribution".to_string()),
    }
}

fn encode_optional_process(payload: &mut Vec<u8>, process: Option<ProcessMarker>) {
    let process = process.unwrap_or(ProcessMarker {
        pid: 0,
        start_time_ticks: 0,
        executable_name: [0; 16],
    });
    payload.extend_from_slice(&process.pid.to_be_bytes());
    append_u64(payload, process.start_time_ticks);
    payload.extend_from_slice(&process.executable_name);
}

fn decode_optional_process(decoder: &mut Decoder<'_>) -> Result<Option<ProcessMarker>, String> {
    let present = decoder.u8()?;
    if decoder.array::<2>()? != [0; 2] {
        return Err("stored OOM victim reserved bytes are non-zero".to_string());
    }
    let process = ProcessMarker {
        pid: decoder.u32()?,
        start_time_ticks: decoder.u64()?,
        executable_name: decoder.array::<16>()?,
    };
    match present {
        0 if process.pid == 0
            && process.start_time_ticks == 0
            && process.executable_name == [0; 16] =>
        {
            Ok(None)
        }
        1 => Ok(Some(process)),
        _ => Err("invalid stored OOM monitored root marker".to_string()),
    }
}

fn encode_process_alert(
    guest_boot_id: GuestBootId,
    process: ProcessMarker,
    sample_started_ms: u64,
    bytes: u64,
    threshold_bytes: u64,
) -> Vec<u8> {
    let mut payload = Vec::with_capacity(60);
    payload.extend_from_slice(guest_boot_id.as_bytes());
    payload.extend_from_slice(&process.pid.to_be_bytes());
    append_u64(&mut payload, process.start_time_ticks);
    payload.extend_from_slice(&process.executable_name);
    append_u64(&mut payload, sample_started_ms);
    append_u64(&mut payload, bytes);
    append_u64(&mut payload, threshold_bytes);
    payload
}

fn append_u64(output: &mut Vec<u8>, value: u64) {
    output.extend_from_slice(&value.to_be_bytes());
}

struct Decoder<'a> {
    payload: &'a [u8],
    offset: usize,
}

impl<'a> Decoder<'a> {
    const fn new(payload: &'a [u8]) -> Self {
        Self { payload, offset: 0 }
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], String> {
        self.take(N)?
            .try_into()
            .map_err(|_| "sandbox alert payload width mismatch".to_string())
    }

    fn u32(&mut self) -> Result<u32, String> {
        self.array::<4>().map(u32::from_be_bytes)
    }

    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.array::<1>()?[0])
    }

    fn u16(&mut self) -> Result<u16, String> {
        self.array::<2>().map(u16::from_be_bytes)
    }

    fn u64(&mut self) -> Result<u64, String> {
        self.array::<8>().map(u64::from_be_bytes)
    }

    fn finish(&self) -> Result<(), String> {
        if self.offset == self.payload.len() {
            Ok(())
        } else {
            Err("sandbox alert payload has trailing bytes".to_string())
        }
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], String> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or_else(|| "sandbox alert payload offset overflow".to_string())?;
        let value = self
            .payload
            .get(self.offset..end)
            .ok_or_else(|| "sandbox alert payload is truncated".to_string())?;
        self.offset = end;
        Ok(value)
    }
}
