//! Container identity resolved from a process's cgroup.
//!
//! `container_id` is the runtime-assigned, human-readable, stable handle for a
//! container. It is 1:1 with the container's pid namespace, but unlike the
//! kernel `NamespaceIdentity` (an opaque, reuse-prone inode) it maps to
//! `docker ps` / image / pod and survives collector restarts.
//!
//! The resolver supports Docker, containerd/Kata and Kubernetes cgroup layouts.
//! The struct keeps `runtime` and `pod_uid` separate so attribution remains
//! explicit without treating a pod UID as a container ID.

use std::fmt;

/// Which container runtime a [`ContainerIdentity`] was parsed from.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ContainerRuntime {
    Docker,
    Containerd,
    K8s,
    Podman,
    Crio,
    Unknown,
}

impl ContainerRuntime {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Docker => "docker",
            Self::Containerd => "containerd",
            Self::K8s => "kubernetes-unknown-cri",
            Self::Podman => "podman",
            Self::Crio => "crio",
            Self::Unknown => "unknown",
        }
    }

    pub fn from_storage_str(raw: &str) -> Option<Self> {
        match raw {
            "docker" => Some(Self::Docker),
            "containerd" => Some(Self::Containerd),
            "kubernetes-unknown-cri" => Some(Self::K8s),
            "podman" => Some(Self::Podman),
            "crio" => Some(Self::Crio),
            "unknown" => Some(Self::Unknown),
            _ => None,
        }
    }
}

/// Full runtime container identity normalized to 32 decoded bytes.
#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct NormalizedContainerId([u8; 32]);

impl NormalizedContainerId {
    pub const BYTE_COUNT: usize = 32;
    pub const HEX_LENGTH: usize = Self::BYTE_COUNT * 2;

    pub const fn from_bytes(bytes: [u8; Self::BYTE_COUNT]) -> Self {
        Self(bytes)
    }

    pub fn from_lower_hex(raw: &str) -> Result<Self, String> {
        if raw.len() != Self::HEX_LENGTH {
            return Err(format!(
                "container ID must contain exactly {} lowercase hexadecimal characters",
                Self::HEX_LENGTH
            ));
        }
        if !raw
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err("container ID must use lowercase hexadecimal characters".to_string());
        }
        let mut bytes = [0_u8; Self::BYTE_COUNT];
        for (index, pair) in raw.as_bytes().chunks_exact(2).enumerate() {
            bytes[index] = (hex_nibble(pair[0]) << 4) | hex_nibble(pair[1]);
        }
        Ok(Self(bytes))
    }

    pub const fn as_bytes(&self) -> &[u8; Self::BYTE_COUNT] {
        &self.0
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

impl fmt::Display for NormalizedContainerId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.to_lower_hex())
    }
}

fn hex_nibble(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        _ => unreachable!("validated lowercase hexadecimal byte"),
    }
}

/// Readable, runtime-assigned container identity.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ContainerIdentity {
    pub runtime: ContainerRuntime,
    pub container_id: String,
    /// K8s pod UID; `None` for plain Docker.
    pub pod_uid: Option<String>,
}

impl ContainerIdentity {
    pub fn new(runtime: ContainerRuntime, container_id: impl Into<String>) -> Self {
        Self {
            runtime,
            container_id: container_id.into(),
            pod_uid: None,
        }
    }

    pub fn container_id(&self) -> &str {
        &self.container_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalized_container_id_requires_exact_lowercase_hex_and_round_trips() {
        let raw = "0123456789abcdef".repeat(4);
        let id = NormalizedContainerId::from_lower_hex(&raw).unwrap();
        assert_eq!(id.to_string(), raw);
        assert_eq!(NormalizedContainerId::from_bytes(*id.as_bytes()), id);
        assert!(NormalizedContainerId::from_lower_hex(&raw[..63]).is_err());
        assert!(NormalizedContainerId::from_lower_hex(&raw.to_uppercase()).is_err());
        assert!(NormalizedContainerId::from_lower_hex(&format!("{}g", &raw[..63])).is_err());
    }

    #[test]
    fn persisted_runtime_names_round_trip() {
        for runtime in [
            ContainerRuntime::Docker,
            ContainerRuntime::Containerd,
            ContainerRuntime::K8s,
            ContainerRuntime::Podman,
            ContainerRuntime::Crio,
            ContainerRuntime::Unknown,
        ] {
            assert_eq!(
                ContainerRuntime::from_storage_str(runtime.as_str()),
                Some(runtime)
            );
        }
    }
}
