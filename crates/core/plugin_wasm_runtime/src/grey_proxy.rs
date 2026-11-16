//! Own one grey proxy child for the lifetime of a WASM control instance.
use plugin_system::{PluginManifest, PluginRuntimeError, PluginWasmAbi};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::os::fd::AsRawFd;
use std::path::Path;
use std::process::{Child, Command, Stdio};

pub(crate) struct GreyProxyProcess {
    child: Child,
}

impl GreyProxyProcess {
    pub(crate) fn prepare(
        manifest: &mut PluginManifest,
        plugin_config: &mut Option<String>,
        storage_path: Option<&Path>,
    ) -> Result<Option<Self>, PluginRuntimeError> {
        let Some(raw) = plugin_config.as_ref() else {
            return Ok(None);
        };
        let mut config: Value = match serde_json::from_str(raw) {
            Ok(value) => value,
            Err(_) => return Ok(None), // Ordinary plugins own their configuration format.
        };
        let Some(grey) = config.get("grey_proxy") else {
            return Ok(None);
        };
        if manifest
            .selected_wasm()
            .is_none_or(|wasm| wasm.abi != PluginWasmAbi::WitComponent)
        {
            return Err(Self::error("grey_proxy requires a WIT component"));
        }
        let http = manifest
            .host
            .http
            .as_ref()
            .ok_or_else(|| Self::error("grey_proxy requires host.http limits"))?;
        let path = storage_path.ok_or_else(|| Self::error("grey_proxy requires SQLite storage"))?;
        let input = json!({"grey_proxy":grey,"storage_path":path,"http":http}).to_string();
        let startup_timeout = i32::try_from(http.timeout_ms)
            .map_err(|_| Self::error("HTTP timeout exceeds child startup range"))?;
        if startup_timeout <= 0 {
            return Err(Self::error("HTTP timeout must be positive"));
        }
        let executable = std::env::current_exe()
            .map_err(Self::error)?
            .with_file_name("actraild-grey-proxy");
        let mut command = Command::new(executable);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        let mut owner = Self {
            child: command.spawn().map_err(Self::error)?,
        };
        let stdin = owner
            .child
            .stdin
            .as_mut()
            .ok_or_else(|| Self::error("missing proxy input"))?;
        writeln!(stdin, "{input}").map_err(Self::error)?;
        let stdout = owner
            .child
            .stdout
            .take()
            .ok_or_else(|| Self::error("missing proxy readiness"))?;
        let mut poll = libc::pollfd {
            fd: stdout.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let ready = unsafe { libc::poll(&mut poll, 1, startup_timeout) };
        if ready <= 0 {
            return Err(Self::error(
                "grey proxy did not become ready within HTTP timeout",
            ));
        }
        let mut endpoint = String::new();
        BufReader::new(stdout)
            .read_line(&mut endpoint)
            .map_err(Self::error)?;
        let endpoint = endpoint.trim();
        if endpoint.is_empty() {
            return Err(Self::error("grey proxy exited before becoming ready"));
        }
        // The child endpoint is runtime-owned; the original external URL was checked by the child.
        manifest
            .host
            .http
            .as_mut()
            .expect("HTTP configuration checked")
            .allowed_urls = vec![endpoint.into()];
        config["endpoint"] = endpoint.into();
        *plugin_config = Some(config.to_string());
        Ok(Some(owner))
    }

    fn error(error: impl std::fmt::Display) -> PluginRuntimeError {
        PluginRuntimeError::new("grey_proxy", error.to_string())
    }
}

impl Drop for GreyProxyProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
