//! Kernel-authenticated Unix-socket peer identity and process ownership checks.

use control_contract::command::ProcessRef;
use control_contract::reply::ControlError;
use ebpf_collector::procfs::{parse_container_identity, resolve_namespaced_pid};
use model_core::ids::TraceId;
use trace_runtime::TraceOwnerPrincipal;
use uds_control_server::PeerCredentials;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PeerPrincipal {
    pub(crate) uid: u32,
    pub(crate) container_id: Option<String>,
    pub(crate) pid_namespace: String,
    pub(crate) host_pid_namespace: bool,
}

/// One principal identity viewed by reference, so `PeerPrincipal` and
/// `TraceOwnerPrincipal` share a single matching rule.
#[derive(Clone, Copy)]
struct PrincipalRef<'a> {
    uid: u32,
    container_id: Option<&'a str>,
    pid_namespace: &'a str,
    host_pid_namespace: bool,
}

fn principals_match(peer: PrincipalRef<'_>, other: PrincipalRef<'_>) -> bool {
    match (peer.container_id, other.container_id) {
        (Some(left), Some(right)) => left == right,
        (None, None) if peer.host_pid_namespace && other.host_pid_namespace => {
            peer.uid == other.uid
        }
        (None, None) if !peer.host_pid_namespace && !other.host_pid_namespace => {
            peer.uid == other.uid && peer.pid_namespace == other.pid_namespace
        }
        _ => false,
    }
}

fn owner_ref(owner: &TraceOwnerPrincipal) -> PrincipalRef<'_> {
    PrincipalRef {
        uid: owner.uid,
        container_id: owner.container_id.as_deref(),
        pid_namespace: &owner.pid_namespace,
        host_pid_namespace: owner.host_pid_namespace,
    }
}

impl PeerPrincipal {
    pub(crate) fn matches(&self, other: &Self) -> bool {
        principals_match(self.as_ref(), other.as_ref())
    }

    pub(crate) fn trace_owner(&self) -> TraceOwnerPrincipal {
        TraceOwnerPrincipal {
            uid: self.uid,
            container_id: self.container_id.clone(),
            pid_namespace: self.pid_namespace.clone(),
            host_pid_namespace: self.host_pid_namespace,
        }
    }

    fn matches_trace_owner(&self, owner: &TraceOwnerPrincipal) -> bool {
        principals_match(self.as_ref(), owner_ref(owner))
    }

    fn as_ref(&self) -> PrincipalRef<'_> {
        PrincipalRef {
            uid: self.uid,
            container_id: self.container_id.as_deref(),
            pid_namespace: &self.pid_namespace,
            host_pid_namespace: self.host_pid_namespace,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PeerIdentity {
    pub(crate) credentials: PeerCredentials,
    pub(crate) principal: PeerPrincipal,
}

impl PeerIdentity {
    pub(crate) fn resolve(credentials: PeerCredentials) -> Result<Self, ControlError> {
        let container_id = process_container_id(credentials.pid)?;
        let pid_namespace = process_pid_namespace(credentials.pid)?;
        let host_pid_namespace = pid_namespace == process_pid_namespace(std::process::id())?;
        Ok(Self {
            credentials,
            principal: PeerPrincipal {
                uid: credentials.uid,
                container_id,
                pid_namespace,
                host_pid_namespace,
            },
        })
    }

    pub(crate) fn is_trusted_host_root(&self) -> bool {
        self.principal.container_id.is_none()
            && self.principal.host_pid_namespace
            && self.credentials.uid == 0
    }

    pub(crate) fn authorize_process_ref(&self, target: &ProcessRef) -> Result<(), ControlError> {
        if self.is_trusted_host_root() {
            return Ok(());
        }
        let process = resolve_namespaced_pid(target.namespace_pid, &target.pid_namespace)
            .map_err(|error| peer_error(format!("resolve target process: {error}")))?;
        let target_pid_namespace = process_pid_namespace(process.pid)?;
        let target = PeerPrincipal {
            uid: process_uid(process.pid)?,
            container_id: process_container_id(process.pid)?,
            host_pid_namespace: target_pid_namespace
                == process_pid_namespace(std::process::id())?,
            pid_namespace: target_pid_namespace,
        };
        if self.principal.matches(&target) {
            Ok(())
        } else {
            Err(peer_error(format!(
                "peer pid={} uid={} container={} cannot act for target pid={} uid={} container={}",
                self.credentials.pid,
                self.credentials.uid,
                display_container(self.principal.container_id.as_deref()),
                process.pid,
                target.uid,
                display_container(target.container_id.as_deref())
            )))
        }
    }

    pub(crate) fn authorize_trace_owner(
        &self,
        trace_id: TraceId,
        owner: &TraceOwnerPrincipal,
    ) -> Result<(), ControlError> {
        if self.is_trusted_host_root() {
            return Ok(());
        }
        if self.principal.matches_trace_owner(owner) {
            Ok(())
        } else {
            Err(peer_error(format!(
                "peer pid={} uid={} is not authorized for trace {trace_id}",
                self.credentials.pid, self.credentials.uid
            )))
        }
    }
}

pub(crate) fn peer_error(message: impl Into<String>) -> ControlError {
    ControlError::new("peer_identity", message)
}

fn process_container_id(pid: u32) -> Result<Option<String>, ControlError> {
    let path = format!("/proc/{pid}/cgroup");
    let content = std::fs::read_to_string(&path)
        .map_err(|error| peer_error(format!("read {path}: {error}")))?;
    Ok(parse_container_identity(&content).map(|identity| identity.container_id))
}

fn process_uid(pid: u32) -> Result<u32, ControlError> {
    let path = format!("/proc/{pid}/status");
    let content = std::fs::read_to_string(&path)
        .map_err(|error| peer_error(format!("read {path}: {error}")))?;
    content
        .lines()
        .find_map(|line| {
            line.strip_prefix("Uid:")
                .and_then(|value| value.split_whitespace().next())
                .and_then(|value| value.parse::<u32>().ok())
        })
        .ok_or_else(|| peer_error(format!("read {path}: missing Uid")))
}

fn process_pid_namespace(pid: u32) -> Result<String, ControlError> {
    let path = format!("/proc/{pid}/ns/pid");
    std::fs::read_link(&path)
        .map(|namespace| namespace.display().to_string())
        .map_err(|error| peer_error(format!("read {path}: {error}")))
}

fn display_container(container_id: Option<&str>) -> &str {
    container_id.unwrap_or("host")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn container_principals_match_by_container_not_uid() {
        let first = PeerPrincipal {
            uid: 0,
            container_id: Some("container-a".to_string()),
            pid_namespace: "pid:[1]".to_string(),
            host_pid_namespace: false,
        };
        let second = PeerPrincipal {
            uid: 1000,
            container_id: Some("container-a".to_string()),
            pid_namespace: "pid:[1]".to_string(),
            host_pid_namespace: false,
        };
        assert!(first.matches(&second));
    }

    #[test]
    fn host_principals_require_same_uid() {
        let first = PeerPrincipal {
            uid: 1000,
            container_id: None,
            pid_namespace: "pid:[1]".to_string(),
            host_pid_namespace: true,
        };
        let same = PeerPrincipal {
            uid: 1000,
            container_id: None,
            pid_namespace: "pid:[1]".to_string(),
            host_pid_namespace: true,
        };
        let other = PeerPrincipal {
            uid: 1001,
            container_id: None,
            pid_namespace: "pid:[1]".to_string(),
            host_pid_namespace: true,
        };
        assert!(first.matches(&same));
        assert!(!first.matches(&other));
    }

    #[test]
    fn different_containers_do_not_match() {
        let first = PeerPrincipal {
            uid: 0,
            container_id: Some("container-a".to_string()),
            pid_namespace: "pid:[1]".to_string(),
            host_pid_namespace: false,
        };
        let second = PeerPrincipal {
            uid: 0,
            container_id: Some("container-b".to_string()),
            pid_namespace: "pid:[2]".to_string(),
            host_pid_namespace: false,
        };
        assert!(!first.matches(&second));
    }

    #[test]
    fn unresolved_isolated_principals_require_same_pid_namespace() {
        let first = PeerPrincipal {
            uid: 0,
            container_id: None,
            pid_namespace: "pid:[10]".to_string(),
            host_pid_namespace: false,
        };
        let same = PeerPrincipal {
            uid: 0,
            container_id: None,
            pid_namespace: "pid:[10]".to_string(),
            host_pid_namespace: false,
        };
        let other = PeerPrincipal {
            uid: 0,
            container_id: None,
            pid_namespace: "pid:[11]".to_string(),
            host_pid_namespace: false,
        };
        assert!(first.matches(&same));
        assert!(!first.matches(&other));
    }
}
