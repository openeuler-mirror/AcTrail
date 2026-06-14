//! Preloaded in-process runtime.

mod config;
mod decision;
mod exec;
mod hook;
mod loader;
mod maps;
mod output;
mod rustls;
mod ssl;

use std::sync::atomic::{AtomicBool, Ordering};

static INITIALIZING: AtomicBool = AtomicBool::new(false);

#[used]
#[unsafe(link_section = ".init_array")]
static TLS_PAYLOAD_SYNC_INIT: extern "C" fn() = init;

extern "C" fn init() {
    if let Err(error) = initialize() {
        output::error_line(&format!("tls_payload_probe_sync error: {error}\n"));
        unsafe {
            libc::_exit(126);
        }
    }
}

fn initialize() -> Result<(), String> {
    if config::get().is_some() {
        return Ok(());
    }
    if INITIALIZING.swap(true, Ordering::AcqRel) {
        return Ok(());
    }
    let result = initialize_once();
    INITIALIZING.store(false, Ordering::Release);
    result
}

fn initialize_once() -> Result<(), String> {
    let Some(bootstrap) = config::RuntimeConfigFactory::from_env()? else {
        return Ok(());
    };
    let initial_plan = bootstrap.initial_plan;
    config::set(bootstrap.config)?;
    if let Some(plan) = initial_plan {
        ssl::install_plan(&plan)?;
    }
    loader::scan_loaded_tls_libraries("init")
}

fn retry_initialize_after_loader_event() {
    if config::get().is_some() {
        return;
    }
    if let Err(error) = initialize() {
        output::error_line(&format!("tls_payload_probe_sync error: {error}\n"));
    }
}

#[cfg(test)]
mod exec_tests {
    use std::ffi::OsString;

    use super::exec::{EnvEntry, merge_java_exec_env};

    #[test]
    fn java_exec_env_merge_preserves_child_env_and_adds_actrail_keys() {
        let current = vec![
            EnvEntry::new("TLS_PAYLOAD_SYNC_TRACE_ID", "42"),
            EnvEntry::new("TLS_PAYLOAD_SYNC_EVENT_SOCKET", "/tmp/actrail.sock"),
            EnvEntry::new(
                "JAVA_TOOL_OPTIONS",
                "-Droot=true -javaagent:/tmp/actrail-java-payload-agent-1234.jar",
            ),
            EnvEntry::new("USER_SETTING", "root"),
        ];
        let child = vec![
            EnvEntry::new("PATH", "/usr/bin"),
            EnvEntry::new("JAVA_TOOL_OPTIONS", "-Dchild=true"),
        ];

        let merged = merge_java_exec_env("/usr/bin/java", &child, &current);

        assert_eq!(merged[0], EnvEntry::new("PATH", "/usr/bin"));
        assert_eq!(
            merged[1],
            EnvEntry::new(
                "JAVA_TOOL_OPTIONS",
                "-Dchild=true -javaagent:/tmp/actrail-java-payload-agent-1234.jar"
            )
        );
        assert!(merged.contains(&EnvEntry::new("TLS_PAYLOAD_SYNC_TRACE_ID", "42")));
        assert!(merged.contains(&EnvEntry::new(
            "TLS_PAYLOAD_SYNC_EVENT_SOCKET",
            "/tmp/actrail.sock"
        )));
        assert!(!merged.contains(&EnvEntry::new("USER_SETTING", "root")));
    }

    #[test]
    fn non_java_exec_env_merge_is_unchanged() {
        let current = vec![EnvEntry::new("TLS_PAYLOAD_SYNC_TRACE_ID", "42")];
        let child = vec![EnvEntry::new("PATH", "/usr/bin")];

        let merged = merge_java_exec_env("/usr/bin/python3", &child, &current);

        assert_eq!(merged, child);
    }

    #[test]
    fn java_exec_env_merge_does_not_duplicate_actrail_agent() {
        let current = vec![EnvEntry::new(
            "JAVA_TOOL_OPTIONS",
            "-javaagent:/tmp/actrail-java-payload-agent-1234.jar",
        )];
        let child = vec![EnvEntry::new(
            "JAVA_TOOL_OPTIONS",
            "-Xmx128m -javaagent:/tmp/actrail-java-payload-agent-1234.jar",
        )];

        let merged = merge_java_exec_env(OsString::from("java.exe"), &child, &current);

        assert_eq!(merged, child);
    }
}
