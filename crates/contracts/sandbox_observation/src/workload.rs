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
mod tests {
    use super::*;

    const ID: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn normalized_container_id_requires_lowercase_hex_and_round_trips() {
        let id = NormalizedContainerId::from_lower_hex(ID).unwrap();
        assert_eq!(id.to_lower_hex(), ID);
        assert_eq!(NormalizedContainerId::from_bytes(*id.as_bytes()), id);
        assert!(NormalizedContainerId::from_lower_hex(&ID[..63]).is_err());
        assert!(NormalizedContainerId::from_lower_hex(&ID.to_uppercase()).is_err());
    }

    #[test]
    fn runtime_codes_are_stable_and_bounded() {
        for runtime in [
            SandboxContainerRuntime::Unknown,
            SandboxContainerRuntime::Docker,
            SandboxContainerRuntime::Containerd,
            SandboxContainerRuntime::KubernetesUnknownCri,
            SandboxContainerRuntime::Podman,
            SandboxContainerRuntime::Crio,
        ] {
            assert_eq!(
                SandboxContainerRuntime::from_code(runtime.code()),
                Some(runtime)
            );
        }
        assert_eq!(SandboxContainerRuntime::from_code(6), None);
        assert_eq!(SandboxContainerRuntime::from_code(255), None);
    }

    #[test]
    fn workload_id_rejects_non_canonical_paths() {
        let boot = GuestBootId::new([0; 16]);
        for path in [
            &b""[..],
            &b"relative"[..],
            &b"/docker//abc"[..],
            &b"/docker/./abc"[..],
            &b"/docker/../abc"[..],
            &b"/docker/"[..],
        ] {
            assert_eq!(
                WorkloadCgroupId::compute(&boot, 1, 2, 3, path),
                Err(WorkloadCgroupIdError::PathNotCanonical),
                "path {path:?} should be rejected"
            );
        }
    }

    #[test]
    fn workload_id_is_deterministic_and_sensitive_to_inputs() {
        let boot = GuestBootId::new([7; 16]);
        let a = WorkloadCgroupId::compute(&boot, 1, 2, 3, b"/docker/abc").unwrap();
        let b = WorkloadCgroupId::compute(&boot, 1, 2, 3, b"/docker/abc").unwrap();
        assert_eq!(a, b);
        let other_path = WorkloadCgroupId::compute(&boot, 1, 2, 3, b"/docker/abd").unwrap();
        assert_ne!(a, other_path);
        let other_boot =
            WorkloadCgroupId::compute(&GuestBootId::new([8; 16]), 1, 2, 3, b"/docker/abc").unwrap();
        assert_ne!(a, other_boot);
    }

    #[test]
    fn snapshot_validation_requires_positive_roots_and_real_marker() {
        let boot = GuestBootId::new([0; 16]);
        let workload_id = WorkloadCgroupId::from_bytes([0; 32]);
        let marker = ProcessMarker {
            pid: 42,
            start_time_ticks: 9,
            executable_name: [0; 16],
        };
        let snapshot = WorkloadCgroupResourceSnapshot {
            guest_boot_id: boot,
            sampled_at_ms: 0,
            workload_id,
            representative_root: marker,
            monitored_root_count: 1,
            runtime: SandboxContainerRuntime::Docker,
            container_id: None,
            counters: WorkloadCgroupCounters::default(),
        };
        assert_eq!(snapshot.validate(), Ok(()));

        let mut zero_count = snapshot;
        zero_count.monitored_root_count = 0;
        assert!(zero_count.validate().is_err());

        let mut zero_marker = snapshot;
        zero_marker.representative_root = ProcessMarker {
            pid: 0,
            start_time_ticks: 0,
            executable_name: [0; 16],
        };
        assert!(zero_marker.validate().is_err());
    }
}
