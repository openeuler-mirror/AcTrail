//! Preloaded in-process runtime.

mod config;
mod decision;
mod flow_control;
mod hook;
mod loader;
mod maps;
mod output;
mod rustls;
mod ssl;
mod tls;

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
    if std::env::var_os(tls_payload_sync::ENV_ENABLED).is_none() {
        return Ok(());
    }
    let audit_namespace = tls::dynamic::binding::is_audit_namespace()?;
    let Some(bootstrap) =
        config::RuntimeConfigFactory::from_env_with_initial_plan(!audit_namespace)?
    else {
        return Ok(());
    };
    let initial_plan = bootstrap.initial_plan;
    config::set(bootstrap.config)?;
    register_exit_flush()?;
    if audit_namespace {
        return Ok(());
    }
    if let Some(plan) = initial_plan {
        ssl::install_plan(&plan)?;
    }
    Ok(())
}

fn register_exit_flush() -> Result<(), String> {
    let result = unsafe { libc::atexit(flush_sync_events) };
    if result == 0 {
        Ok(())
    } else {
        Err("register sync event flush hook failed".to_string())
    }
}

extern "C" fn flush_sync_events() {
    let _ = std::panic::catch_unwind(|| {
        if let Some(config) = config::get() {
            let _ = config.close_event_client();
        }
    });
}

pub(in crate::runtime) fn retry_initialize_after_loader_event() {
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

    use super::loader::exec::{EnvEntry, merge_exec_env};

    #[test]
    fn java_exec_env_merge_preserves_child_env_and_adds_actrail_keys() {
        let current = vec![
            EnvEntry::new("TLS_PAYLOAD_SYNC_TRACE_ID", "42"),
            EnvEntry::new("TLS_PAYLOAD_SYNC_EVENT_SOCKET", "/tmp/actrail.sock"),
            EnvEntry::new(
                "LD_PRELOAD",
                "/opt/actrail/libsync.so:/usr/lib/libcustom.so",
            ),
            EnvEntry::new("LD_AUDIT", "/opt/actrail/libsync.so"),
            EnvEntry::new(
                "JAVA_TOOL_OPTIONS",
                "-Droot=true -javaagent:/tmp/actrail-java-payload-agent-1234.jar",
            ),
            EnvEntry::new("USER_SETTING", "root"),
        ];
        let child = vec![
            EnvEntry::new("PATH", "/usr/bin"),
            EnvEntry::new("LD_PRELOAD", "/usr/lib/libcustom.so"),
            EnvEntry::new("JAVA_TOOL_OPTIONS", "-Dchild=true"),
        ];

        let merged = merge_exec_env("/usr/bin/java", &child, &current);

        assert_eq!(merged[0], EnvEntry::new("PATH", "/usr/bin"));
        assert_eq!(
            merged[2],
            EnvEntry::new(
                "JAVA_TOOL_OPTIONS",
                "-Dchild=true -javaagent:/tmp/actrail-java-payload-agent-1234.jar"
            )
        );
        assert!(merged.contains(&EnvEntry::new(
            "LD_PRELOAD",
            "/usr/lib/libcustom.so:/opt/actrail/libsync.so"
        )));
        assert!(merged.contains(&EnvEntry::new("LD_AUDIT", "/opt/actrail/libsync.so")));
        assert!(merged.contains(&EnvEntry::new("TLS_PAYLOAD_SYNC_TRACE_ID", "42")));
        assert!(merged.contains(&EnvEntry::new(
            "TLS_PAYLOAD_SYNC_EVENT_SOCKET",
            "/tmp/actrail.sock"
        )));
        assert!(!merged.contains(&EnvEntry::new("USER_SETTING", "root")));
    }

    #[test]
    fn non_java_exec_env_merge_preserves_native_runtime_only() {
        let current = vec![
            EnvEntry::new("TLS_PAYLOAD_SYNC_TRACE_ID", "42"),
            EnvEntry::new("LD_PRELOAD", "/opt/actrail/libsync.so"),
            EnvEntry::new("LD_AUDIT", "/opt/actrail/libsync.so"),
            EnvEntry::new(
                "JAVA_TOOL_OPTIONS",
                "-javaagent:/tmp/actrail-java-payload-agent-1234.jar",
            ),
        ];
        let child = vec![EnvEntry::new("PATH", "/usr/bin")];

        let merged = merge_exec_env("/usr/bin/python3", &child, &current);

        assert_eq!(merged[0], EnvEntry::new("PATH", "/usr/bin"));
        assert!(merged.contains(&EnvEntry::new("TLS_PAYLOAD_SYNC_TRACE_ID", "42")));
        assert!(merged.contains(&EnvEntry::new("LD_PRELOAD", "/opt/actrail/libsync.so")));
        assert!(merged.contains(&EnvEntry::new("LD_AUDIT", "/opt/actrail/libsync.so")));
        assert!(
            !merged
                .iter()
                .any(|entry| entry.key() == "JAVA_TOOL_OPTIONS")
        );
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

        let merged = merge_exec_env(OsString::from("java.exe"), &child, &current);

        assert_eq!(merged, child);
    }
}
