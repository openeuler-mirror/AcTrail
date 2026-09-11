//! Shared helpers for runtime-owned host cgroup identity verification.

use std::fs;
use std::io;
use std::path::Path;

use linux_platform::container_cgroup::HostCgroupBindingError;
use model_core::external_cgroup::{ExternalBindingStaleReason, HostBootId};

pub(super) fn read_host_boot_id(procfs_root: &Path) -> io::Result<HostBootId> {
    let raw = fs::read_to_string(procfs_root.join("sys/kernel/random/boot_id"))?;
    parse_host_boot_id(&raw)
}

pub(super) fn stale_reason_for_open_error(
    error: HostCgroupBindingError,
) -> ExternalBindingStaleReason {
    match error {
        HostCgroupBindingError::ContainerIdentityMissing
        | HostCgroupBindingError::ContainerIdentityMismatch
        | HostCgroupBindingError::Malformed(_) => {
            ExternalBindingStaleReason::ContainerIdentityMismatch
        }
        HostCgroupBindingError::RootMoved => ExternalBindingStaleReason::RootMoved,
        HostCgroupBindingError::Io { source, .. }
            if source.kind() == io::ErrorKind::PermissionDenied =>
        {
            ExternalBindingStaleReason::PermissionDenied
        }
        HostCgroupBindingError::Io { .. } => ExternalBindingStaleReason::BoundaryMissing,
        HostCgroupBindingError::Unsupported(_) => {
            ExternalBindingStaleReason::RequiredCounterUnavailable
        }
        HostCgroupBindingError::ExpectedStartTimeMissing
        | HostCgroupBindingError::PidReuse
        | HostCgroupBindingError::MembershipUnstable => {
            ExternalBindingStaleReason::ContainerIdentityMismatch
        }
    }
}

fn parse_host_boot_id(raw: &str) -> io::Result<HostBootId> {
    let value = raw.trim();
    if value.len() != 36
        || value.bytes().enumerate().any(|(index, byte)| match index {
            8 | 13 | 18 | 23 => byte != b'-',
            _ => !byte.is_ascii_hexdigit(),
        })
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "boot ID is not a UUID",
        ));
    }
    let compact = value.bytes().filter(|byte| *byte != b'-');
    let mut bytes = [0_u8; HostBootId::BYTE_COUNT];
    for (index, pair) in compact.collect::<Vec<_>>().chunks_exact(2).enumerate() {
        bytes[index] = (hex_nibble(pair[0])? << 4) | hex_nibble(pair[1])?;
    }
    Ok(HostBootId::from_bytes(bytes))
}

fn hex_nibble(byte: u8) -> io::Result<u8> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid boot ID digit",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boot_id_parser_is_exact() {
        let parsed = parse_host_boot_id("00112233-4455-6677-8899-aabbccddeeff\n").unwrap();
        assert_eq!(
            parsed.as_bytes(),
            &[
                0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
                0xee, 0xff
            ]
        );
        assert!(parse_host_boot_id("not-a-boot-id").is_err());
    }
}
