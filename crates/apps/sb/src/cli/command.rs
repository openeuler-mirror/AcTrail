use std::io;
use std::path::PathBuf;
use std::time::Duration;

use clap::{Args as ClapArgs, Parser, Subcommand};
use sandbox_control::SandboxEndpoint;

use crate::daemon::SbDaemonConfig;

#[derive(Debug, Parser)]
#[command(name = "actrail-sb")]
struct SbArgs {
    #[command(subcommand)]
    command: SbCommand,
}

#[derive(Debug, Subcommand)]
enum SbCommand {
    Daemon(DaemonArgs),
    Connect(ConnectArgs),
    Init(InitArgs),
}

#[derive(Debug, ClapArgs)]
struct DaemonArgs {
    #[arg(long)]
    config: PathBuf,
}

#[derive(Debug, ClapArgs)]
struct ConnectArgs {
    #[arg(long)]
    control_socket: PathBuf,

    #[arg(long)]
    host_cid: u32,

    #[arg(long)]
    port: u32,

    #[arg(long, default_value_t = SbDaemonConfig::DEFAULT_CONTROL_REQUEST_TIMEOUT_MS)]
    request_timeout_ms: u64,

    #[arg(long, default_value_t = SbDaemonConfig::DEFAULT_CONTROL_MAX_FRAME_BYTES)]
    max_frame_bytes: usize,
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
    control_socket: Option<PathBuf>,

    #[arg(long)]
    instance_lock_path: Option<PathBuf>,
}

pub(super) enum SbInvocation {
    Daemon(SbDaemonInvocation),
    Connect(SbConnectInvocation),
    Init(SbInitInvocation),
}

pub(super) struct SbDaemonInvocation {
    pub(super) config_path: PathBuf,
}

pub(super) struct SbConnectInvocation {
    pub(super) control_socket: PathBuf,
    pub(super) request_timeout: Duration,
    pub(super) max_frame_bytes: usize,
    pub(super) endpoint: SandboxEndpoint,
}

pub(super) struct SbInitInvocation {
    pub(super) output: PathBuf,
    pub(super) force: bool,
    pub(super) root_process_names: Vec<String>,
    pub(super) control_socket: Option<PathBuf>,
    pub(super) instance_lock_path: Option<PathBuf>,
}

pub(super) fn parse_args() -> io::Result<SbInvocation> {
    match SbArgs::parse().command {
        SbCommand::Daemon(args) => Ok(SbInvocation::Daemon(SbDaemonInvocation {
            config_path: args.config,
        })),
        SbCommand::Connect(args) => {
            let request_timeout = Duration::from_millis(args.request_timeout_ms);
            if request_timeout.is_zero() || args.max_frame_bytes == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "control request timeout and frame limit must be positive",
                ));
            }
            let endpoint = SandboxEndpoint::new(args.host_cid, args.port)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
            Ok(SbInvocation::Connect(SbConnectInvocation {
                control_socket: args.control_socket,
                request_timeout,
                max_frame_bytes: args.max_frame_bytes,
                endpoint,
            }))
        }
        SbCommand::Init(args) => Ok(SbInvocation::Init(SbInitInvocation {
            output: args.output,
            force: args.force,
            root_process_names: args.root_process_names,
            control_socket: args.control_socket,
            instance_lock_path: args.instance_lock_path,
        })),
    }
}
