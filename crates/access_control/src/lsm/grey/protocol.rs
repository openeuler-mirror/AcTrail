//! Kernel request and reply contracts for the optional LSM decision device.
use super::super::LsmAuditEvent;

pub(super) const REQUEST_BYTES: usize = 5368;

#[derive(Clone, Debug)]
pub(super) struct LsmExecContext {
    pub(super) syscall: String,
    pub(super) requested_path: String,
    pub(super) argv: Vec<String>,
    pub(super) dirfd: Option<i32>,
    pub(super) flags: Option<u64>,
}

#[derive(Clone, Debug)]
pub(super) struct LsmGreyRequest {
    pub(super) request_id: u64,
    pub(super) deadline_boottime_ns: u64,
    pub(super) fallback_deny: bool,
    pub(super) target: String,
    pub(super) event: LsmAuditEvent,
    pub(super) command: Option<LsmExecContext>,
}

impl LsmGreyRequest {
    pub(super) fn decode(raw: &[u8]) -> Result<Self, String> {
        if raw.len() != REQUEST_BYTES {
            return Err(format!(
                "LSM grey request has {} bytes, expected {REQUEST_BYTES}",
                raw.len()
            ));
        }
        let u64_at = |offset| u64::from_ne_bytes(raw[offset..offset + 8].try_into().unwrap());
        let u32_at = |offset| u32::from_ne_bytes(raw[offset..offset + 4].try_into().unwrap());
        let target = String::from_utf8(
            raw[32..160]
                .split(|byte| *byte == 0)
                .next()
                .unwrap()
                .to_vec(),
        )
        .map_err(|error| format!("invalid LSM grey target: {error}"))?;
        Ok(Self {
            request_id: u64_at(0),
            deadline_boottime_ns: u64_at(8),
            fallback_deny: u32_at(24) != 0,
            target,
            event: LsmAuditEvent::decode(&raw[160..992])?,
            command: if u32_at(200) == 4 {
                let syscall = u32_at(992);
                let argc = u32_at(996) as usize;
                if !matches!(syscall, 1 | 2) || argc > 16 {
                    return Err("invalid LSM exec context".into());
                }
                let string = |bytes: &[u8]| {
                    String::from_utf8_lossy(bytes.split(|byte| *byte == 0).next().unwrap())
                        .into_owned()
                };
                Some(LsmExecContext {
                    syscall: if syscall == 1 { "execve" } else { "execveat" }.into(),
                    requested_path: string(&raw[1016..1272]),
                    argv: (0..argc)
                        .map(|index| string(&raw[1272 + index * 256..1272 + (index + 1) * 256]))
                        .collect(),
                    dirfd: (syscall == 2).then(|| u32_at(1000) as i32),
                    flags: (syscall == 2).then(|| u64_at(1008)),
                })
            } else {
                None
            },
        })
    }
}

pub(super) struct LsmGreyReply {
    pub(super) request_id: u64,
    pub(super) decision: u32,
}

impl LsmGreyReply {
    pub(super) fn encode(&self) -> [u8; 16] {
        let mut bytes = [0; 16];
        bytes[..8].copy_from_slice(&self.request_id.to_ne_bytes());
        bytes[8..12].copy_from_slice(&self.decision.to_ne_bytes());
        bytes
    }
}
