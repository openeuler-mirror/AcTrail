//! Connects checkpoint coordination to launch, control commands and the Trace lifecycle.

mod commands;
mod completions;
mod launch;
mod process_control;
mod procfs;
mod service;

pub(crate) use launch::launch_request;
pub(crate) use service::CheckpointService;
