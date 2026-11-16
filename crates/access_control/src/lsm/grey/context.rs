//! Project the kernel request snapshot into the synchronous plugin context.

use super::protocol::LsmGreyRequest;
use plugin_system::{
    CONTROL_CURRENT_CONTEXT_TOKEN, CommandExecutionContext, ControlActorProcessIdentity,
    ControlDecisionRequest, ControlSubject,
};

pub(super) struct GreyContext;

impl GreyContext {
    pub(super) fn project(request: &LsmGreyRequest) -> Result<ControlDecisionRequest, String> {
        let event = &request.event;
        let (subject, operation) = match event.domain {
            1 => (ControlSubject::FileAccess, "open"),
            2 => (ControlSubject::FileAccess, "mkdir"),
            3 => (ControlSubject::FileAccess, "rmdir"),
            4 => (ControlSubject::CommandExecution, "exec"),
            5 => (ControlSubject::NetworkAction, "connect"),
            6 => (ControlSubject::FileAccess, "read"),
            7 => (ControlSubject::FileAccess, "write"),
            8 => (ControlSubject::FileAccess, "delete"),
            9 => (ControlSubject::FileAccess, "attr"),
            10 => (ControlSubject::FileAccess, "rename"),
            11 => (ControlSubject::FileAccess, "link"),
            12 => (ControlSubject::NetworkAction, "send"),
            13 => (ControlSubject::NetworkAction, "bind"),
            14 => (ControlSubject::NetworkAction, "listen"),
            domain => return Err(format!("unknown LSM grey domain {domain}")),
        };
        Ok(ControlDecisionRequest {
            logical_process_id: None,
            decision_id: format!("lsm:{}", request.request_id),
            trace_id: event.trace_id.to_string(),
            subject,
            actor_process_identity: ControlActorProcessIdentity {
                pid: event.observer_tgid,
                // The transport identifies a process, not the triggering thread.
                task_id: None,
                generation: event.start_boottime_ns,
                namespace: std::fs::read_link(format!("/proc/{}/ns/pid", event.observer_tgid))
                    .ok()
                    .map(|path| path.to_string_lossy().into_owned()),
            },
            operation: operation.into(),
            target_summary: event.resource.clone(),
            context_ref: Some(CONTROL_CURRENT_CONTEXT_TOKEN.into()),
            // The snapshot has the accessed path, not the matched rule pattern.
            file_policy_context: None,
            command_execution_context: request.command.as_ref().map(|command| {
                CommandExecutionContext {
                    syscall: command.syscall.clone(),
                    requested_path: command.requested_path.clone(),
                    resolved_path: event.resource.clone(),
                    argv: command.argv.clone(),
                    execveat_dirfd: command.dirfd,
                    execveat_flags: command.flags,
                }
            }),
            network_action_context: None,
        })
    }
}
