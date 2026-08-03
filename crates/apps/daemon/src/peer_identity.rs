//! Kernel-authenticated Unix-socket peer identity and process ownership checks.

use control_contract::command::ProcessRef;
use control_contract::reply::ControlError;
use ebpf_collector::procfs::resolve_namespaced_pid;
use model_core::ids::TraceId;
use model_core::process::ProcessObservation;
use process_identity::ProcessIdentityReader;
use trace_runtime::TraceOwnerPrincipal;
use uds_control_server::PeerCredentials;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PeerPrincipal {
    pub(crate) uid: u32,
    pub(crate) pid_namespace: String,
    pub(crate) mount_namespace: String,
    pub(crate) host_pid_namespace: bool,
    pub(crate) host_mount_namespace: bool,
}

/// One principal identity viewed by reference, so `PeerPrincipal` and
/// `TraceOwnerPrincipal` share a single matching rule.
#[derive(Clone, Copy)]
struct PrincipalRef<'a> {
    uid: u32,
    pid_namespace: &'a str,
    mount_namespace: &'a str,
    host_pid_namespace: bool,
    host_mount_namespace: bool,
}

fn principals_match(peer: PrincipalRef<'_>, other: PrincipalRef<'_>) -> bool {
    if peer.host_pid_namespace != other.host_pid_namespace
        || peer.host_mount_namespace != other.host_mount_namespace
        || peer.pid_namespace != other.pid_namespace
        || peer.mount_namespace != other.mount_namespace
    {
        return false;
    }
    !peer.host_pid_namespace || !peer.host_mount_namespace || peer.uid == other.uid
}

fn owner_ref(owner: &TraceOwnerPrincipal) -> PrincipalRef<'_> {
    PrincipalRef {
        uid: owner.uid,
        pid_namespace: &owner.pid_namespace,
        mount_namespace: &owner.mount_namespace,
        host_pid_namespace: owner.host_pid_namespace,
        host_mount_namespace: owner.host_mount_namespace,
    }
}

impl PeerPrincipal {
    pub(crate) fn matches(&self, other: &Self) -> bool {
        principals_match(self.as_ref(), other.as_ref())
    }

    pub(crate) fn trace_owner(&self) -> TraceOwnerPrincipal {
        TraceOwnerPrincipal {
            uid: self.uid,
            pid_namespace: self.pid_namespace.clone(),
            mount_namespace: self.mount_namespace.clone(),
            host_pid_namespace: self.host_pid_namespace,
            host_mount_namespace: self.host_mount_namespace,
        }
    }

    fn matches_trace_owner(&self, owner: &TraceOwnerPrincipal) -> bool {
        principals_match(self.as_ref(), owner_ref(owner))
    }

    fn as_ref(&self) -> PrincipalRef<'_> {
        PrincipalRef {
            uid: self.uid,
            pid_namespace: &self.pid_namespace,
            mount_namespace: &self.mount_namespace,
            host_pid_namespace: self.host_pid_namespace,
            host_mount_namespace: self.host_mount_namespace,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PeerIdentity {
    pub(crate) credentials: PeerCredentials,
    pub(crate) process: ProcessObservation,
    pub(crate) principal: PeerPrincipal,
}

impl PeerIdentity {
    pub(crate) fn resolve(credentials: PeerCredentials) -> Result<Self, ControlError> {
        let process = ebpf_collector::procfs::ProcfsIdentityReader
            .read_identity(credentials.pid)
            .map_err(|error| {
                peer_error(format!(
                    "resolve peer process identity for pid {}: {error:?}",
                    credentials.pid
                ))
            })?;
        let pid_namespace = process_pid_namespace(credentials.pid)?;
        let mount_namespace = process_mount_namespace(credentials.pid)?;
        let host_pid_namespace = pid_namespace == process_pid_namespace(std::process::id())?;
        let host_mount_namespace = mount_namespace == process_mount_namespace(std::process::id())?;
        Ok(Self {
            credentials,
            process,
            principal: PeerPrincipal {
                uid: credentials.uid,
                pid_namespace,
                mount_namespace,
                host_pid_namespace,
                host_mount_namespace,
            },
        })
    }

    pub(crate) fn is_trusted_host_root(&self) -> bool {
        self.principal.host_pid_namespace
            && self.principal.host_mount_namespace
            && self.credentials.uid == 0
    }

    pub(crate) fn authorize_process_ref(&self, target: &ProcessRef) -> Result<(), ControlError> {
        if self.is_trusted_host_root() {
            return Ok(());
        }
        let process = resolve_namespaced_pid(target.namespace_pid, &target.pid_namespace)
            .map_err(|error| peer_error(format!("resolve target process: {error}")))?;
        let host_pid = process
            .host
            .as_ref()
            .map(|host| host.pid)
            .ok_or_else(|| peer_error("resolved target has no host PID"))?;
        let target_pid_namespace = process_pid_namespace(host_pid)?;
        let target_mount_namespace = process_mount_namespace(host_pid)?;
        let target = PeerPrincipal {
            uid: process_uid(host_pid)?,
            host_pid_namespace: target_pid_namespace == process_pid_namespace(std::process::id())?,
            host_mount_namespace: target_mount_namespace
                == process_mount_namespace(std::process::id())?,
            pid_namespace: target_pid_namespace,
            mount_namespace: target_mount_namespace,
        };
        if self.principal.matches(&target) {
            Ok(())
        } else {
            Err(peer_error(format!(
                "peer pid={} uid={} pid_namespace={} mount_namespace={} cannot act for target pid={} uid={} pid_namespace={} mount_namespace={}",
                self.credentials.pid,
                self.credentials.uid,
                self.principal.pid_namespace,
                self.principal.mount_namespace,
                host_pid,
                target.uid,
                target.pid_namespace,
                target.mount_namespace
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

fn process_mount_namespace(pid: u32) -> Result<String, ControlError> {
    let path = format!("/proc/{pid}/ns/mnt");
    std::fs::read_link(&path)
        .map(|namespace| namespace.display().to_string())
        .map_err(|error| peer_error(format!("read {path}: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isolated_principals_match_by_pid_namespace_across_uids() {
        let first = PeerPrincipal {
            uid: 0,
            pid_namespace: "pid:[1]".to_string(),
            mount_namespace: "mnt:[1]".to_string(),
            host_pid_namespace: false,
            host_mount_namespace: false,
        };
        let second = PeerPrincipal {
            uid: 1000,
            pid_namespace: "pid:[1]".to_string(),
            mount_namespace: "mnt:[1]".to_string(),
            host_pid_namespace: false,
            host_mount_namespace: false,
        };
        assert!(first.matches(&second));
    }

    #[test]
    fn host_principals_require_same_uid() {
        let first = PeerPrincipal {
            uid: 1000,
            pid_namespace: "pid:[1]".to_string(),
            mount_namespace: "mnt:[1]".to_string(),
            host_pid_namespace: true,
            host_mount_namespace: true,
        };
        let same = PeerPrincipal {
            uid: 1000,
            pid_namespace: "pid:[1]".to_string(),
            mount_namespace: "mnt:[1]".to_string(),
            host_pid_namespace: true,
            host_mount_namespace: true,
        };
        let other = PeerPrincipal {
            uid: 1001,
            pid_namespace: "pid:[1]".to_string(),
            mount_namespace: "mnt:[1]".to_string(),
            host_pid_namespace: true,
            host_mount_namespace: true,
        };
        assert!(first.matches(&same));
        assert!(!first.matches(&other));
    }

    #[test]
    fn isolated_principals_with_different_pid_namespaces_do_not_match() {
        let first = PeerPrincipal {
            uid: 0,
            pid_namespace: "pid:[1]".to_string(),
            mount_namespace: "mnt:[1]".to_string(),
            host_pid_namespace: false,
            host_mount_namespace: false,
        };
        let second = PeerPrincipal {
            uid: 0,
            pid_namespace: "pid:[2]".to_string(),
            mount_namespace: "mnt:[2]".to_string(),
            host_pid_namespace: false,
            host_mount_namespace: false,
        };
        assert!(!first.matches(&second));
    }

    #[test]
    fn unresolved_isolated_principals_require_same_pid_namespace() {
        let first = PeerPrincipal {
            uid: 0,
            pid_namespace: "pid:[10]".to_string(),
            mount_namespace: "mnt:[10]".to_string(),
            host_pid_namespace: false,
            host_mount_namespace: false,
        };
        let same = PeerPrincipal {
            uid: 0,
            pid_namespace: "pid:[10]".to_string(),
            mount_namespace: "mnt:[10]".to_string(),
            host_pid_namespace: false,
            host_mount_namespace: false,
        };
        let other = PeerPrincipal {
            uid: 0,
            pid_namespace: "pid:[11]".to_string(),
            mount_namespace: "mnt:[11]".to_string(),
            host_pid_namespace: false,
            host_mount_namespace: false,
        };
        assert!(first.matches(&same));
        assert!(!first.matches(&other));
    }

    #[test]
    fn host_root_trust_depends_only_on_kernel_namespace_and_uid() {
        let peer = PeerIdentity {
            credentials: PeerCredentials {
                pid: 1,
                uid: 0,
                gid: 0,
            },
            process: ProcessObservation::default(),
            principal: PeerPrincipal {
                uid: 0,
                pid_namespace: "pid:[1]".to_string(),
                mount_namespace: "mnt:[1]".to_string(),
                host_pid_namespace: true,
                host_mount_namespace: true,
            },
        };

        assert!(peer.is_trusted_host_root());
    }

    #[test]
    fn host_pid_container_is_not_trusted_as_host_root() {
        let peer = PeerIdentity {
            credentials: PeerCredentials {
                pid: 2,
                uid: 0,
                gid: 0,
            },
            process: ProcessObservation::default(),
            principal: PeerPrincipal {
                uid: 0,
                pid_namespace: "pid:[1]".to_string(),
                mount_namespace: "mnt:[container]".to_string(),
                host_pid_namespace: true,
                host_mount_namespace: false,
            },
        };

        assert!(!peer.is_trusted_host_root());
    }

    #[test]
    fn shared_pid_namespaces_are_isolated_by_mount_namespace() {
        let first = PeerPrincipal {
            uid: 0,
            pid_namespace: "pid:[shared]".to_string(),
            mount_namespace: "mnt:[1]".to_string(),
            host_pid_namespace: false,
            host_mount_namespace: false,
        };
        let second = PeerPrincipal {
            uid: 0,
            pid_namespace: "pid:[shared]".to_string(),
            mount_namespace: "mnt:[2]".to_string(),
            host_pid_namespace: false,
            host_mount_namespace: false,
        };

        assert!(!first.matches(&second));
    }
}
