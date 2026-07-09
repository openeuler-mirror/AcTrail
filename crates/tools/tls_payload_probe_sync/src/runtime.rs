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
#[cfg(target_env = "musl")]
mod unwind_stubs;

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
    loader::scan_loaded_tls_libraries("init")?;
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
    if let Some(config) = config::get() {
        let _ = config.close_event_client();
    }
}

pub(in crate::runtime) fn retry_initialize_after_loader_event() {
    if config::get().is_some() {
        return;
    }
    if let Err(error) = initialize() {
        output::error_line(&format!("tls_payload_probe_sync error: {error}\n"));
    }
}
