//! Seccomp connect request capture and typed network action context.

use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};

use control_contract::reply::ControlError;
use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;
use plugin_system::{ControlActorProcessIdentity, NetworkActionContext};
use process_identity::ProcessIdentityManager;

use crate::services::identity::{ControlActorIdentityResolver, ResolvedTraceProcess};
use crate::services::seccomp_notify::read_process_bytes;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct NetworkRemote {
    endpoint: SocketAddr,
    address_family: &'static str,
    remote_address: String,
    remote_port: u16,
    ipv6_scope_id: u32,
}

impl NetworkRemote {
    pub(super) fn read(
        pid: u32,
        sockaddr_ptr: u64,
        sockaddr_len: u64,
    ) -> Result<Option<Self>, ControlError> {
        if sockaddr_ptr == 0 {
            return Ok(None);
        }
        let read_len = usize::try_from(sockaddr_len.min(28)).map_err(|error| {
            ControlError::new(
                "network_control_sockaddr",
                format!("sockaddr len overflow: {error}"),
            )
        })?;
        let Some(bytes) = read_process_bytes(pid, sockaddr_ptr, read_len)? else {
            return Ok(None);
        };
        if bytes.len() < 2 {
            return Err(ControlError::new(
                "network_control_sockaddr",
                "sockaddr is shorter than the address-family field",
            ));
        }
        let family = u16::from_ne_bytes([bytes[0], bytes[1]]);
        if family == libc::AF_INET as u16 {
            return Self::ipv4(&bytes).map(Some);
        }
        if family == libc::AF_INET6 as u16 {
            return Self::ipv6(&bytes).map(Some);
        }
        Ok(None)
    }

    pub(super) fn endpoint(&self) -> SocketAddr {
        self.endpoint
    }

    pub(super) fn address_family(&self) -> &'static str {
        self.address_family
    }

    pub(super) fn address(&self) -> &str {
        &self.remote_address
    }

    pub(super) fn port(&self) -> u16 {
        self.remote_port
    }

    pub(super) fn ipv6_scope_id(&self) -> u32 {
        self.ipv6_scope_id
    }

    fn ipv4(bytes: &[u8]) -> Result<Self, ControlError> {
        if bytes.len() < 8 {
            return Err(ControlError::new(
                "network_control_sockaddr",
                "short AF_INET sockaddr",
            ));
        }
        let port = u16::from_be_bytes([bytes[2], bytes[3]]);
        let address = Ipv4Addr::new(bytes[4], bytes[5], bytes[6], bytes[7]);
        Ok(Self {
            endpoint: SocketAddr::from((address, port)),
            address_family: "ipv4",
            remote_address: address.to_string(),
            remote_port: port,
            ipv6_scope_id: 0,
        })
    }

    fn ipv6(bytes: &[u8]) -> Result<Self, ControlError> {
        if bytes.len() < 24 {
            return Err(ControlError::new(
                "network_control_sockaddr",
                "short AF_INET6 sockaddr",
            ));
        }
        let port = u16::from_be_bytes([bytes[2], bytes[3]]);
        let mut raw_addr = [0_u8; 16];
        raw_addr.copy_from_slice(&bytes[8..24]);
        let address = Ipv6Addr::from(raw_addr);
        let ipv6_scope_id = bytes
            .get(24..28)
            .map(|scope| u32::from_ne_bytes([scope[0], scope[1], scope[2], scope[3]]))
            .unwrap_or(0);
        Ok(Self {
            endpoint: SocketAddr::from((address, port)),
            address_family: "ipv6",
            remote_address: address.to_string(),
            remote_port: port,
            ipv6_scope_id,
        })
    }
}

#[derive(Clone, Debug)]
pub(super) struct NetworkConnectContext {
    trace_id: TraceId,
    process: ProcessIdentity,
    actor: ControlActorProcessIdentity,
    fd: u64,
    remote: NetworkRemote,
}

impl NetworkConnectContext {
    pub(super) fn capture(
        listener_trace_id: TraceId,
        resolved: ResolvedTraceProcess,
        process_registry: &ProcessIdentityManager,
        task_id: u32,
        fd: u64,
        remote: NetworkRemote,
    ) -> Result<Self, ControlError> {
        if resolved.trace_id != listener_trace_id {
            return Err(ControlError::new(
                "network_control_identity",
                format!(
                    "listener trace {listener_trace_id} resolved connect process into trace {}",
                    resolved.trace_id
                ),
            ));
        }
        let mut actor =
            ControlActorIdentityResolver::new(process_registry).resolve(resolved.process)?;
        actor.task_id = Some(task_id);
        Ok(Self {
            trace_id: listener_trace_id,
            process: resolved.process,
            actor,
            fd,
            remote,
        })
    }

    pub(super) fn trace_id(&self) -> TraceId {
        self.trace_id
    }

    pub(super) fn process(&self) -> ProcessIdentity {
        self.process
    }

    pub(super) fn actor(&self) -> ControlActorProcessIdentity {
        self.actor.clone()
    }

    pub(super) fn process_generation(&self) -> u64 {
        self.actor.generation
    }

    pub(super) fn fd(&self) -> u64 {
        self.fd
    }

    pub(super) fn remote(&self) -> &NetworkRemote {
        &self.remote
    }

    pub(super) fn endpoint(&self) -> SocketAddr {
        self.remote.endpoint()
    }

    pub(super) fn target_summary(&self) -> String {
        format!(
            "remote={} family={} fd={}",
            self.remote.endpoint(),
            self.remote.address_family(),
            self.fd
        )
    }

    pub(super) fn action_context(&self) -> NetworkActionContext {
        NetworkActionContext {
            syscall: "connect".to_string(),
            fd: self.fd,
            address_family: self.remote.address_family().to_string(),
            remote_address: self.remote.address().to_string(),
            remote_port: self.remote.port(),
            ipv6_scope_id: self.remote.ipv6_scope_id(),
        }
    }
}
