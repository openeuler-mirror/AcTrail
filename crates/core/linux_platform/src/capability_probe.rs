//! Local runtime capability probes for actrailctl launch environments.

use std::path::Path;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapabilityStatus {
    pub name: &'static str,
    pub available: bool,
    pub detail: String,
}

impl CapabilityStatus {
    pub fn ok(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            available: true,
            detail: detail.into(),
        }
    }

    pub fn unavailable(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            available: false,
            detail: detail.into(),
        }
    }
}

pub fn probe_unix_socket(path: &Path) -> CapabilityStatus {
    match std::os::unix::net::UnixStream::connect(path) {
        Ok(_) => CapabilityStatus::ok("unix_socket", format!("connected {}", path.display())),
        Err(error) => CapabilityStatus::unavailable(
            "unix_socket",
            format!("connect {}: {error}", path.display()),
        ),
    }
}

pub fn probe_no_new_privs() -> CapabilityStatus {
    let result = unsafe { libc::prctl(libc::PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0) };
    if result < 0 {
        return CapabilityStatus::unavailable(
            "no_new_privs",
            format!("prctl(PR_GET_NO_NEW_PRIVS): {}", std::io::Error::last_os_error()),
        );
    }
    CapabilityStatus::ok("no_new_privs", format!("enabled={result}"))
}
