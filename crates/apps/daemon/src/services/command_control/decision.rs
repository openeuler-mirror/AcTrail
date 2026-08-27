//! Exec notification capture and tracee-namespace path resolution.

use std::path::{Component, Path, PathBuf};

use control_contract::reply::ControlError;
use linux_platform::process_seccomp::KernelProcessSyscall;
use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;
use plugin_system::{
    CommandExecutionContext, CommandPolicyDecision, CommandPolicyRuleDraft,
    ControlActorProcessIdentity,
};
use process_identity::ProcessIdentityManager;
use sha2::{Digest, Sha256};

use crate::services::identity::{ControlActorIdentityResolver, ResolvedTraceProcess};
use crate::services::seccomp_notify::{read_c_string, read_process_bytes};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CommandSyscall {
    Execve,
    Execveat,
}

impl CommandSyscall {
    fn from_notification(notification: &libc::seccomp_notif) -> Option<Self> {
        match KernelProcessSyscall::from_number(notification.data.nr) {
            Some(KernelProcessSyscall::Execve) => Some(Self::Execve),
            Some(KernelProcessSyscall::Execveat) => Some(Self::Execveat),
            _ => None,
        }
    }

    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Execve => "execve",
            Self::Execveat => "execveat",
        }
    }

    pub(super) fn notification_name(notification: &libc::seccomp_notif) -> Option<&'static str> {
        Self::from_notification(notification).map(Self::as_str)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ExecNotificationContext {
    trace_id: TraceId,
    task_id: u32,
    process: ProcessIdentity,
    actor: ControlActorProcessIdentity,
    syscall: CommandSyscall,
    requested_path: String,
    resolved_path: PathBuf,
    argv_pointer: u64,
    argv: Option<Vec<String>>,
    argv_digest: Option<String>,
    execveat_dirfd: Option<i32>,
    execveat_flags: Option<u64>,
}

impl ExecNotificationContext {
    pub(super) fn capture(
        listener_trace_id: TraceId,
        resolved: ResolvedTraceProcess,
        process_registry: &ProcessIdentityManager,
        notification: &libc::seccomp_notif,
        path_max_bytes: u32,
    ) -> Result<Option<Self>, String> {
        let Some(syscall) = CommandSyscall::from_notification(notification) else {
            return Ok(None);
        };
        if resolved.trace_id != listener_trace_id {
            return Err(format!(
                "listener trace {listener_trace_id} received pid {} owned by trace {}",
                notification.pid, resolved.trace_id
            ));
        }
        if !resolved.is_capturable() {
            return Err(format!(
                "pid {} is not capturable in trace {listener_trace_id}",
                notification.pid
            ));
        }
        let process = resolved.process;
        let mut actor = ControlActorIdentityResolver::new(process_registry)
            .resolve(process)
            .map_err(|error| format!("{}: {}", error.code, error.message))?;
        actor.task_id = Some(notification.pid);
        let path_pointer = match syscall {
            CommandSyscall::Execve => notification.data.args[0],
            CommandSyscall::Execveat => notification.data.args[1],
        };
        let remote_path = read_c_string(notification.pid, path_pointer, path_max_bytes)
            .map_err(|error| format!("{}: {}", error.code, error.message))?;
        if remote_path.truncated {
            return Err(format!(
                "{} executable path exceeds {} bytes or lacks NUL termination",
                syscall.as_str(),
                path_max_bytes
            ));
        }
        let requested_path = remote_path
            .value
            .ok_or_else(|| format!("{} executable path is unreadable", syscall.as_str()))?;
        let (execveat_dirfd, execveat_flags, argv_pointer) = match syscall {
            CommandSyscall::Execve => (None, None, notification.data.args[1]),
            CommandSyscall::Execveat => (
                Some(notification.data.args[0] as i64 as i32),
                Some(notification.data.args[4]),
                notification.data.args[2],
            ),
        };
        let resolved_path = CommandPath::resolve(
            notification.pid,
            syscall,
            &requested_path,
            execveat_dirfd,
            execveat_flags,
        )?;
        Ok(Some(Self {
            trace_id: listener_trace_id,
            task_id: notification.pid,
            process,
            actor,
            syscall,
            requested_path,
            resolved_path,
            argv_pointer,
            argv: None,
            argv_digest: None,
            execveat_dirfd,
            execveat_flags,
        }))
    }

    pub(super) fn snapshot_argv(
        &mut self,
        max_count: u32,
        max_arg_bytes: u32,
        max_total_bytes: u32,
    ) -> Result<(), String> {
        if self.argv.is_some() {
            return Ok(());
        }
        let snapshot = ArgvSnapshotReader::new(
            self.task_id,
            self.argv_pointer,
            max_count,
            max_arg_bytes,
            max_total_bytes,
        )
        .read()?;
        self.argv_digest = Some(snapshot.digest);
        self.argv = Some(snapshot.argv);
        Ok(())
    }

    pub(super) fn trace_id(&self) -> TraceId {
        self.trace_id
    }

    pub(crate) fn process(&self) -> ProcessIdentity {
        self.process
    }

    pub(crate) fn actor_pid(&self) -> u32 {
        self.actor.pid
    }

    pub(super) fn process_generation(&self) -> u64 {
        self.actor.generation
    }

    pub(super) fn actor(&self) -> ControlActorProcessIdentity {
        self.actor.clone()
    }

    pub(super) fn syscall(&self) -> CommandSyscall {
        self.syscall
    }

    pub(crate) fn requested_path(&self) -> &str {
        &self.requested_path
    }

    pub(crate) fn syscall_name(&self) -> &'static str {
        self.syscall.as_str()
    }

    pub(crate) fn resolved_path(&self) -> &Path {
        &self.resolved_path
    }

    pub(crate) fn argv(&self) -> &[String] {
        self.argv.as_deref().unwrap_or_default()
    }

    pub(super) fn arguments(&self) -> &[String] {
        self.argv().get(1..).unwrap_or_default()
    }

    pub(super) fn argv_was_snapshotted(&self) -> bool {
        self.argv.is_some()
    }

    pub(super) fn argv_digest(&self) -> Option<&str> {
        self.argv_digest.as_deref()
    }

    pub(crate) fn execveat_dirfd(&self) -> Option<i32> {
        self.execveat_dirfd
    }

    pub(crate) fn execveat_flags(&self) -> Option<u64> {
        self.execveat_flags
    }

    pub(super) fn command_execution_context(&self) -> CommandExecutionContext {
        CommandExecutionContext {
            syscall: self.syscall.as_str().to_string(),
            requested_path: self.requested_path.clone(),
            resolved_path: self.resolved_path.display().to_string(),
            argv: self.argv().to_vec(),
            execveat_dirfd: self.execveat_dirfd,
            execveat_flags: self.execveat_flags,
        }
    }
}

pub(super) struct CommandPath;

impl CommandPath {
    pub(super) fn normalize_absolute(raw: &str) -> Result<PathBuf, String> {
        if raw.is_empty() {
            return Err("command executable must not be empty".to_string());
        }
        if raw.contains('\0') {
            return Err("command executable contains NUL".to_string());
        }
        let path = Path::new(raw);
        if !path.is_absolute() {
            return Err(format!("command executable {raw} must be absolute"));
        }
        let mut normalized = PathBuf::from("/");
        for component in path.components() {
            match component {
                Component::RootDir | Component::CurDir => {}
                Component::ParentDir => {
                    normalized.pop();
                }
                Component::Normal(part) => normalized.push(part),
                Component::Prefix(_) => {
                    return Err(format!("command executable {raw} must be a Unix path"));
                }
            }
        }
        Ok(normalized)
    }

    fn resolve(
        pid: u32,
        syscall: CommandSyscall,
        requested: &str,
        dirfd: Option<i32>,
        flags: Option<u64>,
    ) -> Result<PathBuf, String> {
        if Path::new(requested).is_absolute() {
            return Self::normalize_absolute(requested);
        }
        let empty_path = syscall == CommandSyscall::Execveat
            && requested.is_empty()
            && flags.is_some_and(|value| value & libc::AT_EMPTY_PATH as u64 != 0);
        let base = if empty_path {
            Self::read_proc_link(
                pid,
                "fd",
                dirfd.ok_or_else(|| "execveat AT_EMPTY_PATH lacks dirfd".to_string())?,
            )?
        } else {
            if requested.is_empty() {
                return Err(format!("{} executable path is empty", syscall.as_str()));
            }
            match (syscall, dirfd) {
                (CommandSyscall::Execve, _) | (_, Some(libc::AT_FDCWD)) => {
                    Self::read_proc_link(pid, "cwd", 0)?
                }
                (CommandSyscall::Execveat, Some(fd)) => Self::read_proc_link(pid, "fd", fd)?,
                (CommandSyscall::Execveat, None) => {
                    return Err("execveat notification lacks dirfd".to_string());
                }
            }
        };
        if empty_path {
            return Self::normalize_proc_link(base);
        }
        let joined = base.join(requested);
        Self::normalize_absolute(&joined.display().to_string())
    }

    fn read_proc_link(pid: u32, kind: &str, value: i32) -> Result<PathBuf, String> {
        let link = if kind == "cwd" {
            PathBuf::from(format!("/proc/{pid}/cwd"))
        } else {
            PathBuf::from(format!("/proc/{pid}/fd/{value}"))
        };
        let target = std::fs::read_link(&link)
            .map_err(|error| format!("resolve {} failed: {error}", link.display()))?;
        Self::normalize_proc_link(target)
    }

    fn normalize_proc_link(target: PathBuf) -> Result<PathBuf, String> {
        let raw = target.display().to_string();
        if raw.ends_with(" (deleted)") {
            return Err(format!(
                "command executable {} is deleted and cannot be mapped into the tracee namespace",
                target.display()
            ));
        }
        Self::normalize_absolute(&raw)
    }
}

pub(super) enum CommandGrantScope {
    Exact(PathBuf),
    Recursive(PathBuf),
}

pub(super) struct CommandRuleDraftValidator;

impl CommandRuleDraftValidator {
    pub(super) fn validate_id(rule_id: &str) -> Result<(), String> {
        if rule_id.trim().is_empty() {
            return Err("command rule id must not be empty".to_string());
        }
        if rule_id.chars().any(char::is_whitespace) {
            return Err(format!(
                "command rule id {rule_id} must not contain whitespace"
            ));
        }
        Ok(())
    }

    pub(super) fn validate_shape(draft: &CommandPolicyRuleDraft) -> Result<(), String> {
        match (draft.decision, draft.gray_target.as_deref()) {
            (CommandPolicyDecision::Default, _) => {
                Err("command rule decision cannot be default".to_string())
            }
            (CommandPolicyDecision::Gray, None) => {
                Err("gray command rule requires gray_target".to_string())
            }
            (CommandPolicyDecision::Gray, Some(target)) if target.trim().is_empty() => {
                Err("gray command rule target must not be empty".to_string())
            }
            (CommandPolicyDecision::Allow | CommandPolicyDecision::Deny, Some(_)) => {
                Err("allow/deny command rule must not include gray_target".to_string())
            }
            _ => Ok(()),
        }
    }
}

impl CommandGrantScope {
    pub(super) fn parse(raw: &str) -> Result<Self, String> {
        let recursive_base = raw.strip_suffix("/**");
        let check = recursive_base.unwrap_or(raw);
        if check.contains('*') {
            return Err(format!(
                "command policy grant {raw} may only use /** as its final suffix"
            ));
        }
        match recursive_base {
            Some("") => Err("command policy recursive grant base must not be empty".to_string()),
            Some(base) => CommandPath::normalize_absolute(base).map(Self::Recursive),
            None => CommandPath::normalize_absolute(raw).map(Self::Exact),
        }
    }

    pub(super) fn contains(&self, executable: &Path) -> bool {
        match self {
            Self::Exact(path) => path == executable,
            Self::Recursive(base) => executable.starts_with(base),
        }
    }
}

struct ArgvSnapshot {
    argv: Vec<String>,
    digest: String,
}

struct ArgvSnapshotReader {
    pid: u32,
    argv_pointer: u64,
    max_count: usize,
    max_arg_bytes: usize,
    max_total_bytes: usize,
}

impl ArgvSnapshotReader {
    fn new(
        pid: u32,
        argv_pointer: u64,
        max_count: u32,
        max_arg_bytes: u32,
        max_total_bytes: u32,
    ) -> Self {
        Self {
            pid,
            argv_pointer,
            max_count: max_count as usize,
            max_arg_bytes: max_arg_bytes as usize,
            max_total_bytes: max_total_bytes as usize,
        }
    }

    fn read(&self) -> Result<ArgvSnapshot, String> {
        if self.argv_pointer == 0 {
            return Ok(Self::finish(Vec::new()));
        }
        let pointer_size = std::mem::size_of::<usize>();
        let mut argv = Vec::new();
        let mut total_bytes = 0_usize;
        for index in 0..=self.max_count {
            let offset = index
                .checked_mul(pointer_size)
                .and_then(|value| self.argv_pointer.checked_add(value as u64))
                .ok_or_else(|| "command argv pointer overflow".to_string())?;
            let raw = read_process_bytes(self.pid, offset, pointer_size)
                .map_err(control_error_message)?
                .ok_or_else(|| "command argv pointer table became unreadable".to_string())?;
            if raw.len() != pointer_size {
                return Err(format!(
                    "command argv pointer table short read: expected {pointer_size}, got {}",
                    raw.len()
                ));
            }
            let pointer = usize::from_ne_bytes(
                raw.try_into()
                    .map_err(|_| "command argv pointer width mismatch".to_string())?,
            ) as u64;
            if pointer == 0 {
                return Ok(Self::finish(argv));
            }
            if index == self.max_count {
                return Err(format!(
                    "command argv exceeds configured count limit {}",
                    self.max_count
                ));
            }
            let read_limit = self
                .max_arg_bytes
                .checked_add(1)
                .ok_or_else(|| "command argv argument limit overflow".to_string())?;
            let raw = read_process_bytes(self.pid, pointer, read_limit)
                .map_err(control_error_message)?
                .ok_or_else(|| format!("command argv[{index}] became unreadable"))?;
            let end = raw.iter().position(|byte| *byte == 0).ok_or_else(|| {
                format!(
                    "command argv[{index}] exceeds configured argument limit {} or lacks NUL termination",
                    self.max_arg_bytes
                )
            })?;
            if end > self.max_arg_bytes {
                return Err(format!(
                    "command argv[{index}] exceeds configured argument limit {}",
                    self.max_arg_bytes
                ));
            }
            total_bytes = total_bytes
                .checked_add(end)
                .ok_or_else(|| "command argv total byte count overflow".to_string())?;
            if total_bytes > self.max_total_bytes {
                return Err(format!(
                    "command argv exceeds configured total byte limit {}",
                    self.max_total_bytes
                ));
            }
            argv.push(
                String::from_utf8(raw[..end].to_vec())
                    .map_err(|error| format!("command argv[{index}] is not UTF-8: {error}"))?,
            );
        }
        Err("command argv is not NUL terminated".to_string())
    }

    fn finish(argv: Vec<String>) -> ArgvSnapshot {
        let mut digest = Sha256::new();
        for arg in &argv {
            digest.update((arg.len() as u64).to_le_bytes());
            digest.update(arg.as_bytes());
        }
        ArgvSnapshot {
            argv,
            digest: format!("{:x}", digest.finalize()),
        }
    }
}

fn control_error_message(error: ControlError) -> String {
    format!("{}: {}", error.code, error.message)
}
