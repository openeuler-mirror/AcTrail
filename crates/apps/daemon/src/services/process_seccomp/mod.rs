//! Process-control seccomp observation service.

mod clone_flags;
mod procfs;
mod remote_args;
mod service;
mod syscall;

pub(crate) use service::{
    PROCESS_SECCOMP_COLLECTOR_NAME, ProcessSeccompObservation, ProcessSeccompService,
};
