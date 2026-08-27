use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use alert_proxy::{AlertProxyBootstrap, AlertProxyConfig, report_startup_failure};
use clap::Parser;

static STOP_REQUESTED: AtomicBool = AtomicBool::new(false);

#[derive(Parser)]
struct Args {
    #[arg(long)]
    config: PathBuf,
}

extern "C" fn request_stop(_: libc::c_int) {
    STOP_REQUESTED.store(true, Ordering::Release);
}

fn main() {
    if let Err(error) = run() {
        report_startup_failure(&error);
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args = Args::parse();
    let config = AlertProxyConfig::load(&args.config)?;
    unsafe {
        libc::signal(libc::SIGINT, request_stop as libc::sighandler_t);
        libc::signal(libc::SIGTERM, request_stop as libc::sighandler_t);
    }
    let mut runtime = AlertProxyBootstrap::start(config)?;
    while !STOP_REQUESTED.load(Ordering::Acquire) {
        std::thread::sleep(Duration::from_millis(100));
    }
    runtime.shutdown()
}
