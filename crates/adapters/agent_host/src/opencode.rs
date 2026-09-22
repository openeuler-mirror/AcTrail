use std::ffi::{OsStr, OsString};
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
    ) -> Result<(), String> {
        let existing = envs
            .iter()
            .rev()
            .find(|(key, _)| key == "OPENCODE_CONFIG_CONTENT")
            .map(|(_, value)| value.clone())
            .or_else(|| std::env::var_os("OPENCODE_CONFIG_CONTENT"));
        let content = self.config_content(existing.as_deref())?;
        envs.retain(|(key, _)| key != "OPENCODE_CONFIG_CONTENT");
        envs.extend([
            ("ACTRAIL_AGENT_LIFECYCLE_ENABLED".into(), "true".into()),
            ("ACTRAIL_TRACE_ID".into(), trace_id.get().to_string().into()),
            (
                "ACTRAIL_CONTROL_SOCKET".into(),
                control_socket.as_os_str().to_os_string(),
            ),
            ("OPENCODE_CONFIG_CONTENT".into(), content.into()),
        ]);
        Ok(())
    }

    fn config_content(&self, existing: Option<&OsStr>) -> Result<String, String> {
        let mut config: serde_json::Value = match existing {
            None => serde_json::json!({}),
            Some(value) if value.is_empty() => serde_json::json!({}),
            Some(value) => {
                let text = value
                    .to_str()
                    .ok_or("OPENCODE_CONFIG_CONTENT must be UTF-8")?;
                serde_json::from_str(text)
                    .map_err(|error| format!("parse OPENCODE_CONFIG_CONTENT: {error}"))?
            }
        };
        let object = config
            .as_object_mut()
            .ok_or("OPENCODE_CONFIG_CONTENT must be a JSON object")?;
        let plugins = object
            .entry("plugin")
            .or_insert_with(|| serde_json::json!([]))
            .as_array_mut()
            .ok_or("OPENCODE_CONFIG_CONTENT.plugin must be an array")?;
        let entry = self.plugin_dir.join(REQUIRED_FILES[0]);
        let url = url::Url::from_file_path(&entry).map_err(|_| {
            format!(
                "convert OpenCode plugin path to file URL: {}",
                entry.display()
            )
        })?;
        // Preserve tuple specs (including their options) as well as string specs.
        if !plugins.iter().any(|spec| {
            spec.as_str().or_else(|| spec.as_array()?.first()?.as_str()) == Some(url.as_str())
        }) {
            plugins.push(serde_json::Value::String(url.into()));
        }
        serde_json::to_string(&config)
            .map_err(|error| format!("serialize OPENCODE_CONFIG_CONTENT: {error}"))
    }
}

