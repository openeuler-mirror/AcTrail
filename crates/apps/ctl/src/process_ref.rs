//! Caller-namespace process references for control-plane requests.

use std::path::PathBuf;

use control_contract::command::ProcessRef;
use model_core::process::NamespaceIdentity;

pub(crate) fn process_ref(pid: u32) -> Result<ProcessRef, String> {
    Ok(ProcessRef::new(pid, read_pid_namespace(pid)?))
}

fn read_pid_namespace(pid: u32) -> Result<NamespaceIdentity, String> {
    let path = PathBuf::from(format!("/proc/{pid}/ns/pid"));
    std::fs::read_link(&path)
        .map(|value| NamespaceIdentity::new(value.display().to_string()))
        .map_err(|error| format!("read {}: {}", path.display(), error))
}
