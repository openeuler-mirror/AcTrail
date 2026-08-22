use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use clap::{Args as ClapArgs, Parser, Subcommand};
use sb::{SandboxAgentBootstrap, SbConfig, SbConfigOverrides};

static STOP: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Parser)]
#[command(args_conflicts_with_subcommands = true)]
struct Args {
    #[arg(long)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    Init(InitArgs),
}

#[derive(Debug, ClapArgs)]
struct InitArgs {
    #[arg(long)]
    output: PathBuf,

    #[arg(long)]
    force: bool,

    #[arg(long = "root-process-name")]
    root_process_names: Vec<String>,

    #[arg(long)]
    host_cid: Option<u32>,

    #[arg(long)]
    port: Option<u32>,

    #[arg(long)]
    instance_lock_path: Option<PathBuf>,
}

extern "C" fn request_stop(_: libc::c_int) {
    STOP.store(true, Ordering::Release);
}

fn main() {
    if let Err(error) = run() {
        eprintln!("actrail-sb: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    match args.command {
        Some(Command::Init(init)) => return init.write(),
        None => {}
    }
    let config_path = args.config.ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "--config is required unless the init subcommand is used",
        )
    })?;
    let config = SbConfig::load(&config_path)?;
    // SAFETY: handlers only perform an atomic store, which is async-signal-safe.
    unsafe {
        libc::signal(libc::SIGINT, request_stop as libc::sighandler_t);
        libc::signal(libc::SIGTERM, request_stop as libc::sighandler_t);
    }
    let mut process = SandboxAgentBootstrap::start(config)?;
    println!(
        "actrail-sb ready sb_id={}",
        process.agent().snapshot().sb_id
    );
    while !STOP.load(Ordering::Acquire) {
        std::thread::sleep(Duration::from_millis(100));
    }
    process.shutdown()?;
    Ok(())
}

impl InitArgs {
    fn write(self) -> Result<(), Box<dyn std::error::Error>> {
        let mut overrides = SbConfigOverrides::new();
        if !self.root_process_names.is_empty() {
            overrides = overrides.with_root_process_names(self.root_process_names);
        }
        if let Some(host_cid) = self.host_cid {
            overrides = overrides.with_host_cid(host_cid);
        }
        if let Some(port) = self.port {
            overrides = overrides.with_port(port);
        }
        if let Some(path) = self.instance_lock_path {
            overrides = overrides.with_instance_lock_path(path);
        }
        SbConfig::write_default(&self.output, overrides, self.force)?;
        println!("wrote actrail-sb config {}", self.output.display());
        Ok(())
    }
}
