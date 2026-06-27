//! Caller-namespace process references for control-plane requests.

use std::path::PathBuf;

use control_contract::command::ProcessRef;
use model_core::process::NamespaceIdentity;

pub(crate) fn process_ref(pid: u32) -> Result<ProcessRef, String> {
    Ok(ProcessRef::new(
        read_namespace_pid(pid)?,
        read_pid_namespace(pid)?,
    ))
}

fn read_pid_namespace(pid: u32) -> Result<NamespaceIdentity, String> {
    let path = PathBuf::from(format!("/proc/{pid}/ns/pid"));
    std::fs::read_link(&path)
        .map(|value| NamespaceIdentity::new(value.display().to_string()))
        .map_err(|error| format!("read {}: {}", path.display(), error))
}

fn read_namespace_pid(pid: u32) -> Result<u32, String> {
    let path = PathBuf::from(format!("/proc/{pid}/status"));
    let raw = std::fs::read_to_string(&path)
        .map_err(|error| format!("read {}: {}", path.display(), error))?;
    raw.lines()
        .find_map(|line| {
            line.strip_prefix("NSpid:").and_then(|value| {
                value
                    .split_whitespace()
                    .last()
                    .and_then(|raw| raw.parse::<u32>().ok())
            })
        })
        .ok_or_else(|| format!("read {}: missing NSpid", path.display()))
}
