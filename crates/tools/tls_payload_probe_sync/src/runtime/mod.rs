//! Preloaded in-process runtime.

mod config;
mod decision;
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
    let Some(config) = config::RuntimeConfigFactory::from_env()? else {
        return Ok(());
    };
    let points = config.points().to_vec();
    config::set(config)?;
    ssl::install_hooks(&points)
}

fn retry_initialize_after_loader_event() {
    if config::get().is_some() {
        return;
    }
    if let Err(error) = initialize() {
        output::error_line(&format!("tls_payload_probe_sync error: {error}\n"));
    }
}
