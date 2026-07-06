//! Launch-time Java agent environment injection for JSSE payload capture.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::PathBuf;

use config_core::daemon::PayloadTlsConfig;

mod artifact {
    include!(concat!(env!("OUT_DIR"), "/java_agent_artifact.rs"));
}

const JAVA_TOOL_OPTIONS: &str = "JAVA_TOOL_OPTIONS";

pub(super) fn maybe_append_java_agent_env(
    required: bool,
    envs: &mut Vec<(OsString, OsString)>,
) -> Result<(), String> {
    if !required {
        return Ok(());
    }
    let agent_option = materialized_java_agent_option()?;
    append_java_tool_options(envs, &agent_option);
    Ok(())
}

pub(super) fn java_agent_env_required(config: &PayloadTlsConfig) -> bool {
    should_inject_java_agent(config)
}

fn should_inject_java_agent(config: &PayloadTlsConfig) -> bool {
    config.enabled && config.capture_backend.is_sync() && config.java_agent_enabled
}

fn materialized_java_agent_option() -> Result<OsString, String> {
    let jar = materialize_java_agent()?;
    Ok(OsString::from(format!("-javaagent:{}", jar.display())))
}

fn materialize_java_agent() -> Result<PathBuf, String> {
    let bytes = artifact::JAVA_PAYLOAD_AGENT_JAR
        .ok_or_else(|| java_agent_unavailable_error(artifact::JAVA_PAYLOAD_AGENT_BUILD_ERROR))?;
    let path = std::env::temp_dir().join(format!(
        "actrail-java-payload-agent-{:016x}.jar",
        fnv1a64(bytes)
    ));
    if fs::read(&path).is_ok_and(|existing| existing == bytes) {
        return Ok(path);
    }
    fs::write(&path, bytes)
        .map_err(|error| format!("write Java payload agent {}: {error}", path.display()))?;
    Ok(path)
}

fn java_agent_unavailable_error(build_error: Option<&str>) -> String {
    match build_error {
        Some(error) if !error.is_empty() => {
            format!("embedded Java payload agent jar is unavailable ({error})")
        }
        _ => "embedded Java payload agent jar is unavailable".to_string(),
    }
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn append_java_tool_options(envs: &mut Vec<(OsString, OsString)>, agent_option: &OsStr) {
    let existing = envs
        .iter()
        .rev()
        .find(|(key, _)| key == OsStr::new(JAVA_TOOL_OPTIONS))
        .map(|(_, value)| value.clone())
        .or_else(|| std::env::var_os(JAVA_TOOL_OPTIONS));
    envs.push((
        OsString::from(JAVA_TOOL_OPTIONS),
        merged_java_tool_options(existing.as_deref(), agent_option),
    ));
}

fn merged_java_tool_options(existing: Option<&OsStr>, agent_option: &OsStr) -> OsString {
    let mut value = OsString::new();
    if let Some(existing) = existing.filter(|value| !value.is_empty()) {
        if contains_java_tool_option(existing, agent_option) {
            return existing.to_os_string();
        }
        value.push(existing);
        value.push(" ");
    }
    value.push(agent_option);
    value
}

fn contains_java_tool_option(existing: &OsStr, agent_option: &OsStr) -> bool {
    let Some(existing) = existing.to_str() else {
        return false;
    };
    let Some(agent_option) = agent_option.to_str() else {
        return false;
    };
    existing
        .split_whitespace()
        .any(|token| token == agent_option)
}

#[cfg(test)]
mod tests {
    use std::ffi::{OsStr, OsString};
    use std::path::PathBuf;

    use config_core::daemon::{
        DisabledOrPath, PayloadRedactionPolicy, PayloadTlsCaptureBackend, PayloadTlsConfig,
        PayloadTlsLibrary, PayloadTlsLibraryPath, PayloadTlsResolver, PayloadTlsSource,
        PayloadTlsSyncRuntimeLibraryPath,
    };
    use payload_capability::DEFAULT_TLS_SYNC_FLOW_UNKNOWN_STREAM_BYTES;

    use super::{
        JAVA_TOOL_OPTIONS, append_java_tool_options, java_agent_env_required,
        java_agent_unavailable_error, merged_java_tool_options, should_inject_java_agent,
    };

    #[test]
    fn java_tool_options_appends_agent_to_existing_value() {
        let value = merged_java_tool_options(
            Some(OsStr::new("-Dexample=true")),
            OsStr::new("-javaagent:/tmp/agent.jar"),
        );

        assert_eq!(
            value,
            OsString::from("-Dexample=true -javaagent:/tmp/agent.jar")
        );
    }

    #[test]
    fn java_tool_options_sets_agent_when_empty() {
        let value = merged_java_tool_options(None, OsStr::new("-javaagent:/tmp/agent.jar"));

        assert_eq!(value, OsString::from("-javaagent:/tmp/agent.jar"));
    }

    #[test]
    fn java_tool_options_does_not_duplicate_existing_agent_option() {
        let value = merged_java_tool_options(
            Some(OsStr::new("-Dexample=true -javaagent:/tmp/agent.jar")),
            OsStr::new("-javaagent:/tmp/agent.jar"),
        );

        assert_eq!(
            value,
            OsString::from("-Dexample=true -javaagent:/tmp/agent.jar")
        );
    }

    #[test]
    fn append_java_tool_options_prefers_existing_launch_env() {
        let mut envs = vec![(
            OsString::from(JAVA_TOOL_OPTIONS),
            OsString::from("-Xmx128m"),
        )];

        append_java_tool_options(&mut envs, OsStr::new("-javaagent:/tmp/agent.jar"));

        assert_eq!(
            envs.last(),
            Some(&(
                OsString::from(JAVA_TOOL_OPTIONS),
                OsString::from("-Xmx128m -javaagent:/tmp/agent.jar"),
            ))
        );
    }

    #[test]
    fn java_agent_injection_is_config_scoped_for_root_and_child_java() {
        let config = tls_sync_config(true);

        assert!(java_agent_env_required(&config));
    }

    #[test]
    fn java_agent_injection_respects_config_flag() {
        let config = tls_sync_config(false);

        assert!(!should_inject_java_agent(&config));
        assert!(!java_agent_env_required(&config));
    }

    #[test]
    fn unavailable_agent_error_includes_build_failure() {
        let error = java_agent_unavailable_error(Some("javac failed"));

        assert!(error.contains("embedded Java payload agent jar is unavailable"));
        assert!(error.contains("javac failed"));
    }

    fn tls_sync_config(java_agent_enabled: bool) -> PayloadTlsConfig {
        PayloadTlsConfig {
            enabled: true,
            capture_backend: PayloadTlsCaptureBackend::TlsSync,
            source: PayloadTlsSource::Auto,
            resolver: PayloadTlsResolver::Auto,
            library: PayloadTlsLibrary::Auto,
            library_path: PayloadTlsLibraryPath::Auto,
            binary_path: DisabledOrPath::Disabled,
            pattern_path: DisabledOrPath::Disabled,
            max_segment_bytes: 4095,
            max_operation_bytes: 4 * 1024 * 1024,
            ring_buffer_bytes: 1024 * 1024,
            pending_operation_max_entries: 4096,
            seccomp_syscalls: Vec::new(),
            diagnostics_enabled: false,
            retention_max_bytes_per_trace: 10 * 1024 * 1024,
            redaction_policy: PayloadRedactionPolicy::AuthorizationHeader,
            sync_runtime_library_path: PayloadTlsSyncRuntimeLibraryPath::Path(PathBuf::from(
                "/tmp/libtls_payload_probe_sync.so",
            )),
            sync_event_socket_path: PathBuf::from("/tmp/actrail-test-tls-sync.sock"),
            sync_socket_mode: 0o660,
            sync_match_limit: 8,
            sync_flow_control_enabled: true,
            sync_flow_sniff_bytes: 65536,
            sync_flow_max_header_bytes: 16384,
            sync_flow_large_transfer_bytes: 1048576,
            sync_flow_unknown_stream_bytes: DEFAULT_TLS_SYNC_FLOW_UNKNOWN_STREAM_BYTES,
            sync_flow_h2_data_probe_bytes: 65536,
            java_agent_enabled,
        }
    }
}
