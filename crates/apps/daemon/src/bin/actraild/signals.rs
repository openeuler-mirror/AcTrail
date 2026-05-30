//! Minimal signal bridge for foreground daemon shutdown.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

static SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);

pub fn install_shutdown_handlers() -> Result<(), String> {
    install_handler(libc::SIGTERM)?;
    install_handler(libc::SIGINT)
}

pub fn shutdown_requested() -> bool {
    SHUTDOWN_REQUESTED.load(Ordering::SeqCst)
}

extern "C" fn handle_shutdown_signal(_: libc::c_int) {
    SHUTDOWN_REQUESTED.store(true, Ordering::SeqCst);
}

fn install_handler(signal: libc::c_int) -> Result<(), String> {
    let previous = unsafe {
        libc::signal(
            signal,
            handle_shutdown_signal as *const () as libc::sighandler_t,
        )
    };
    if previous == libc::SIG_ERR {
        return Err(format!(
            "install signal handler {signal}: {}",
            io::Error::last_os_error()
        ));
    }
    Ok(())
}
