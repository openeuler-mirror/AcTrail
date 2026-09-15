//! OpenCode idle-detection adapter integration for launch.

use std::ffi::OsString;
use std::path::Path;

const ENV_IDLE_DETECTION_ENABLED: &str = "ACTRAIL_IDLE_DETECTION_ENABLED";
const ENV_OPENCODE_CONFIG_DIR: &str = "OPENCODE_CONFIG_DIR";

const REQUIRED_PLUGIN_FILES: &[&str] = &[
    "plugins/actrail-idle-plugin.js",
    "lib/actrail-idle-adapter.js",
    "lib/actrail-control-socket-client.js",
    "lib/actrail-constants.js",
    "lib/actrail-opencode-utils.js",
    "lib/actrail-opencode-permission-client.js",
];

pub(super) fn validate_plugin_dir(plugin_dir: Option<&Path>) -> Result<(), String> {
    let Some(plugin_dir) = plugin_dir else {
        return Ok(());
    };
    for relative_path in REQUIRED_PLUGIN_FILES {
        let path = plugin_dir.join(relative_path);
        if !path.is_file() {
            return Err(format!(
                "OpenCode idle plugin is incomplete: required file is missing: {}",
                path.display()
            ));
        }
    }
    Ok(())
}

pub(super) fn push_launch_env(envs: &mut Vec<(OsString, OsString)>, plugin_dir: Option<&Path>) {
    envs.push((
        OsString::from(ENV_IDLE_DETECTION_ENABLED),
        OsString::from(plugin_dir.is_some().to_string()),
    ));
    if let Some(plugin_dir) = plugin_dir {
        envs.push((
            OsString::from(ENV_OPENCODE_CONFIG_DIR),
            plugin_dir.as_os_str().to_os_string(),
        ));
    }
}
