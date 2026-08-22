use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use clap::{Args as ClapArgs, Parser, Subcommand, ValueEnum};
use vsock_gateway::{GatewayAppConfig, GatewayBackend, GatewayBootstrap, GatewayConfigOverrides};

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

    #[arg(long, value_enum)]
    backend: Option<BackendArg>,

    #[arg(long)]
    socket_path: Option<PathBuf>,

    #[arg(long)]
    cid: Option<u32>,

    #[arg(long)]
    port: Option<u32>,

    #[arg(long)]
    daemon_address: Option<SocketAddr>,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum BackendArg {
    Native,
    CloudHypervisor,
}

extern "C" fn request_stop(_: libc::c_int) {
    STOP.store(true, Ordering::Release);
}

fn main() {
    if let Err(error) = run() {
        eprintln!("actrail-vsock-gateway: {error}");
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
    let config = GatewayAppConfig::load(&config_path)?;
    // SAFETY: handlers only perform an atomic store, which is async-signal-safe.
    unsafe {
        libc::signal(libc::SIGINT, request_stop as libc::sighandler_t);
        libc::signal(libc::SIGTERM, request_stop as libc::sighandler_t);
    }
    let mut runtime = GatewayBootstrap::start(config)?;
    let snapshot = runtime.snapshot();
    println!("gateway ready gateway_id={}", snapshot.gateway_id);
    while !STOP.load(Ordering::Acquire) {
        std::thread::sleep(Duration::from_millis(100));
    }
    runtime.shutdown()?;
    Ok(())
}

impl InitArgs {
    fn write(self) -> Result<(), Box<dyn std::error::Error>> {
        let mut overrides = GatewayConfigOverrides::new();
        if let Some(backend) = self.backend {
            overrides = overrides.with_backend(match backend {
                BackendArg::Native => GatewayBackend::Native,
                BackendArg::CloudHypervisor => GatewayBackend::CloudHypervisor,
            });
        }
        if let Some(socket_path) = self.socket_path {
            overrides = overrides.with_socket_path(socket_path);
        }
        if let Some(cid) = self.cid {
            overrides = overrides.with_cid(cid);
        }
        if let Some(port) = self.port {
            overrides = overrides.with_port(port);
        }
        if let Some(daemon_address) = self.daemon_address {
            overrides = overrides.with_daemon_address(daemon_address);
        }
        GatewayAppConfig::write_default(&self.output, overrides, self.force)?;
        println!(
            "wrote actrail-vsock-gateway config {}",
            self.output.display()
        );
        Ok(())
    }
}
