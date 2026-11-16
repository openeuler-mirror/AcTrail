//! Guest workload cgroup resource observations (Track B contract).

use std::fmt;

use sha2::{Digest, Sha256};

use crate::{GuestBootId, ProcessMarker};

/// Domain prefix hashed at the start of every workload identity v1.
const WORKLOAD_ID_DOMAIN: &[u8] = b"actrail.workload-cgroup.v1\0";

/// Maximum length in bytes of a canonical unified relative cgroup path.
const MAX_CANONICAL_PATH_BYTES: usize = 4096;

/// SHA-256 workload identity. Identifies one guest cgroup boundary without
/// exposing or trusting the raw path across the wire.
#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct WorkloadCgroupId([u8; 32]);

impl WorkloadCgroupId {
    pub const BYTE_COUNT: usize = 32;

    pub const fn from_bytes(bytes: [u8; Self::BYTE_COUNT]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; Self::BYTE_COUNT] {
        &self.0
    }

    /// Computes the v1 workload identity over the canonical inputs.
    ///
    /// The canonical path is local hash input only: it starts with `/`, has no
    /// empty, `.`, or `..` components, and is at most [`MAX_CANONICAL_PATH_BYTES`]
    /// bytes long.
    pub fn compute(
        guest_boot_id: &GuestBootId,
        cgroup2_mount_id: u64,
        st_dev: u64,
        st_ino: u64,
        canonical_path: &[u8],
    ) -> Result<Self, WorkloadCgroupIdError> {
        validate_canonical_path(canonical_path)?;
        let path_length =
            u16::try_from(canonical_path.len()).map_err(|_| WorkloadCgroupIdError::PathTooLong)?;
        let mut hasher = Sha256::new();
        hasher.update(WORKLOAD_ID_DOMAIN);
        hasher.update(guest_boot_id.as_bytes());
        hasher.update(cgroup2_mount_id.to_be_bytes());
        hasher.update(st_dev.to_be_bytes());
        hasher.update(st_ino.to_be_bytes());
        hasher.update(path_length.to_be_bytes());
        hasher.update(canonical_path);
        Ok(Self(hasher.finalize().into()))
    }
}

impl fmt::Debug for WorkloadCgroupId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("WorkloadCgroupId")
            .field(&self.0)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkloadCgroupIdError {
    PathTooLong,
    PathNotCanonical,
}

impl fmt::Display for WorkloadCgroupIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PathTooLong => formatter.write_str("canonical cgroup path exceeds 4096 bytes"),
            Self::PathNotCanonical => {
                formatter.write_str("canonical cgroup path must start with '/' and contain no empty, '.' or '..' components")
            }
        }
    }
}

impl std::error::Error for WorkloadCgroupIdError {}

fn validate_canonical_path(path: &[u8]) -> Result<(), WorkloadCgroupIdError> {
    if path.len() > MAX_CANONICAL_PATH_BYTES {
        return Err(WorkloadCgroupIdError::PathTooLong);
    }
    if path.is_empty() || path[0] != b'/' {
        return Err(WorkloadCgroupIdError::PathNotCanonical);
    }
    let mut components = path.split(|byte| *byte == b'/');
    // Drop the empty component produced by the leading '/'.
    components.next();
    for component in components {
        if component.is_empty() || component == b"." || component == b".." {
            return Err(WorkloadCgroupIdError::PathNotCanonical);
        }
    }
    Ok(())
}

/// Runtime codes carried by a workload observation.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum SandboxContainerRuntime {
    Unknown,
    Docker,
    Containerd,
    KubernetesUnknownCri,
    Podman,
    Crio,
}

impl SandboxContainerRuntime {
    pub const fn code(self) -> u8 {
        match self {
            Self::Unknown => 0,
            Self::Docker => 1,
            Self::Containerd => 2,
            Self::KubernetesUnknownCri => 3,
            Self::Podman => 4,
            Self::Crio => 5,
        }
    }

    pub fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::Unknown),
            1 => Some(Self::Docker),
            2 => Some(Self::Containerd),
            3 => Some(Self::KubernetesUnknownCri),
            4 => Some(Self::Podman),
            5 => Some(Self::Crio),
            _ => None,
        }
    }
}

/// Normalized container identity: 32 decoded bytes rendering as 64 lowercase
/// hexadecimal characters.
#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct NormalizedContainerId([u8; 32]);

impl NormalizedContainerId {
    pub const BYTE_COUNT: usize = 32;
    pub const HEX_LENGTH: usize = Self::BYTE_COUNT * 2;

    pub const fn from_bytes(bytes: [u8; Self::BYTE_COUNT]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; Self::BYTE_COUNT] {
        &self.0
    }

    pub fn from_lower_hex(raw: &str) -> Result<Self, String> {
        if raw.len() != Self::HEX_LENGTH
            || !raw
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(
                "container ID must be exactly 64 lowercase hexadecimal characters".to_string(),
            );
        }
        let mut bytes = [0_u8; Self::BYTE_COUNT];
        for (index, pair) in raw.as_bytes().chunks_exact(2).enumerate() {
            bytes[index] = (hex_nibble(pair[0]) << 4) | hex_nibble(pair[1]);
        }
        Ok(Self(bytes))
    }

    pub fn to_lower_hex(self) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut output = String::with_capacity(Self::HEX_LENGTH);
        for byte in self.0 {
            output.push(HEX[usize::from(byte >> 4)] as char);
            output.push(HEX[usize::from(byte & 0x0f)] as char);
        }
        output
    }
}

impl fmt::Debug for NormalizedContainerId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("NormalizedContainerId")
            .field(&self.to_lower_hex())
            .finish()
    }
}

fn hex_nibble(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        _ => unreachable!("validated lowercase hexadecimal byte"),
    }
}

/// Counter values for one guest workload cgroup.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct WorkloadCgroupCounters {
    pub memory_current_bytes: u64,
    pub memory_peak_bytes: Option<u64>,
    pub memory_anon_bytes: Option<u64>,
    pub memory_file_bytes: Option<u64>,
    pub memory_swap_current_bytes: Option<u64>,
    pub memory_low: Option<u64>,
    pub memory_high: Option<u64>,
    pub memory_max: Option<u64>,
    pub memory_oom: Option<u64>,
    pub memory_oom_kill: Option<u64>,
    pub memory_oom_group_kill: Option<u64>,
    pub cpu_usage_usec: Option<u64>,
    pub cpu_user_usec: Option<u64>,
    pub cpu_system_usec: Option<u64>,
    pub cpu_nr_throttled: Option<u64>,
    pub cpu_throttled_usec: Option<u64>,
    pub io_read_bytes: Option<u64>,
    pub io_write_bytes: Option<u64>,
    pub pids_current: Option<u64>,
    pub pids_peak: Option<u64>,
}

/// One read-only guest workload cgroup observation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkloadCgroupResourceSnapshot {
    pub guest_boot_id: GuestBootId,
    pub sampled_at_ms: u64,
    pub workload_id: WorkloadCgroupId,
    pub representative_root: ProcessMarker,
    pub monitored_root_count: u32,
    pub runtime: SandboxContainerRuntime,
    pub container_id: Option<NormalizedContainerId>,
    pub counters: WorkloadCgroupCounters,
}

impl WorkloadCgroupResourceSnapshot {
    /// `monitored_root_count` must be positive; the representative root must be
    /// a real process marker rather than the zero placeholder.
    pub fn validate(&self) -> Result<(), String> {
        if self.monitored_root_count == 0 {
            return Err("workload cgroup monitored root count must be positive".to_string());
        }
        if self.representative_root.pid == 0 && self.representative_root.start_time_ticks == 0 {
            return Err("workload cgroup representative root must be a real process".to_string());
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "../../../../tests/unit/crates/contracts/sandbox_observation/workload/tests.rs"]
mod tests;
