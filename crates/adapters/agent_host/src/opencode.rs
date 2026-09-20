use std::ffi::OsString;
use std::path::{Path, PathBuf};

use model_core::ids::TraceId;

use crate::config::OpenCodeConfig;

const REQUIRED_FILES: &[&str] = &[
    "plugins/actrail-lifecycle-plugin.js",
    "lib/actrail-lifecycle-adapter.js",
    "lib/actrail-control-socket-client.js",
    "lib/actrail-constants.js",
    "lib/actrail-opencode-utils.js",
    "lib/actrail-opencode-permission-client.js",
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OpenCodeLaunch {
    plugin_dir: PathBuf,
}

impl OpenCodeLaunch {
    pub(crate) fn matches(argv: &[String]) -> bool {
        argv.first()
            .and_then(|command| Path::new(command).file_name())
            .is_some_and(|name| name == "opencode")
    }

    pub(crate) fn prepare(config: &OpenCodeConfig) -> Result<Self, String> {
        let plugin_dir = config
            .plugin_dir
            .as_ref()
            .ok_or_else(|| "opencode.plugin_dir is required".to_string())?;
        let plugin_dir = std::fs::canonicalize(plugin_dir).map_err(|error| {
            format!(
                "resolve OpenCode plugin directory {}: {error}",
                plugin_dir.display()
            )
        })?;
        for name in REQUIRED_FILES {
            let path = plugin_dir.join(name);
            if !path.is_file() {
                return Err(format!(
                    "OpenCode lifecycle plugin is incomplete: {}",
                    path.display()
                ));
            }
        }
        Ok(Self { plugin_dir })
    }

    pub(crate) fn append_env(
        &self,
        trace_id: TraceId,
        control_socket: &Path,
        envs: &mut Vec<(OsString, OsString)>,
    ) {
        envs.extend([
            ("ACTRAIL_AGENT_LIFECYCLE_ENABLED".into(), "true".into()),
            ("ACTRAIL_TRACE_ID".into(), trace_id.get().to_string().into()),
            (
                "ACTRAIL_CONTROL_SOCKET".into(),
                control_socket.as_os_str().to_os_string(),
            ),
            (
                "OPENCODE_CONFIG_DIR".into(),
                self.plugin_dir.as_os_str().to_os_string(),
            ),
        ]);
    }
}
