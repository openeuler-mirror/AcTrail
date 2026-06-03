//! Preloaded in-process runtime.

mod config;
mod decision;
mod hook;
mod maps;
mod output;
mod rustls;
mod ssl;

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
    let Some(config) = config::RuntimeConfigFactory::from_env()? else {
        return Ok(());
    };
    let points = config.points().to_vec();
    config::set(config)?;
    ssl::install_hooks(&points)
}
