use std::ffi::OsString;
use std::path::Path;

use model_core::ids::TraceId;

use crate::config::AgentHostConfig;
use crate::opencode::OpenCodeLaunch;

/// A validated launch integration; absence of configuration means no injection.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AgentLaunchIntegration {
    opencode: Option<OpenCodeLaunch>,
}

impl AgentLaunchIntegration {
    pub fn prepare(config: &AgentHostConfig, argv: &[String]) -> Result<Self, String> {
        let opencode = if config.opencode.enabled && OpenCodeLaunch::matches(argv) {
            Some(OpenCodeLaunch::prepare(&config.opencode)?)
        } else {
            None
        };
        Ok(Self { opencode })
    }

    pub fn append_env(
        &self,
        trace_id: TraceId,
        control_socket: &Path,
        envs: &mut Vec<(OsString, OsString)>,
    ) {
        if let Some(opencode) = &self.opencode {
            opencode.append_env(trace_id, control_socket, envs);
        }
    }
}
