//! Command-line input shapes for the control application.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use clap::{ArgAction, Args, Parser, Subcommand};
use config_core::daemon::{
    DEFAULT_OPERATOR_CONFIG_PATH, OperatorConfig, PayloadSocketSeccompSyscall,
    PayloadTlsSeccompSyscall, ProcessSeccompSyscall,
};
use control_contract::selector::TraceSelector;
use model_core::ids::{ProfileName, RequestId, TraceId, TraceName};

use crate::clean::CleanArtifacts;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CtlInvocation {
    pub socket_path: PathBuf,
    pub request_id: RequestId,
    pub command: CtlCommand,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CtlCommand {
    TrackAdd {
        root_pid: u32,
        display_name: TraceName,
        profile_name: ProfileName,
        tags: BTreeSet<String>,
    },
    Launch {
        display_name: TraceName,
        profile_name: ProfileName,
        tags: BTreeSet<String>,
        payload_tls_enabled: bool,
        payload_tls_seccomp_syscalls: Vec<PayloadTlsSeccompSyscall>,
        payload_socket_enabled: bool,
        payload_socket_seccomp_syscalls: Vec<PayloadSocketSeccompSyscall>,
        payload_socket_max_segment_bytes: u32,
        process_seccomp_enabled: bool,
        process_seccomp_syscalls: Vec<ProcessSeccompSyscall>,
        seccomp_notify_reserved_listener_fd: u32,
        argv: Vec<String>,
    },
    TrackRemove {
        selector: TraceSelector,
    },
    ListTraces {
        selector: Option<TraceSelector>,
    },
    Clean {
        artifacts: CleanArtifacts,
    },
    Doctor,
}

pub fn parse_args(args: impl IntoIterator<Item = String>) -> Result<CtlInvocation, String> {
    let cli = CtlCli::try_parse_from(std::iter::once("actrailctl".to_string()).chain(args))
        .unwrap_or_else(|error| error.exit());
    cli.into_invocation()
}

#[derive(Clone, Debug, Parser)]
#[command(name = "actrailctl", about = "Control a running AcTrail daemon")]
struct CtlCli {
    #[arg(long = "config", global = true, value_name = "PATH")]
    config_path: Option<PathBuf>,

    #[arg(long = "socket-path", global = true, value_name = "PATH")]
    socket_path: Option<PathBuf>,

    #[arg(long = "request-id", global = true, value_name = "ID")]
    request_id: Option<u64>,

    #[command(subcommand)]
    command: CtlCommandArgs,
}

impl CtlCli {
    fn into_invocation(self) -> Result<CtlInvocation, String> {
        let needs_profile = matches!(
            &self.command,
            CtlCommandArgs::TrackAdd(args) if args.profile_name.is_none()
        ) || matches!(
            &self.command,
            CtlCommandArgs::Launch(args) if args.profile_name.is_none()
        );
        let explicit_config = self.config_path.is_some();
        let config_path = self.config_path.unwrap_or_else(default_config_path);
        let needs_config = explicit_config
            || self.socket_path.is_none()
            || needs_profile
            || self.command.is_clean();
        let operator_config = load_operator_config(needs_config, &config_path)?;
        let socket_path = self
            .socket_path
            .or_else(|| {
                operator_config
                    .as_ref()
                    .map(|config| config.socket_path.clone())
            })
            .ok_or_else(|| {
                "missing --socket-path and operator config was not loaded".to_string()
            })?;
        let request_id = match self.request_id {
            Some(raw) => RequestId::new(raw),
            None => generated_request_id()?,
        };
        Ok(CtlInvocation {
            socket_path,
            request_id,
            command: self.command.into_command(operator_config.as_ref())?,
        })
    }
}

#[derive(Clone, Debug, Subcommand)]
enum CtlCommandArgs {
    #[command(about = "Attach a trace to an existing root process")]
    TrackAdd(TrackAddArgs),
    #[command(about = "Attach this actrailctl process, then run a child command")]
    Launch(LaunchArgs),
    #[command(about = "Remove a trace by selector")]
    TrackRemove(SelectorArgs),
    #[command(about = "List traces")]
    ListTraces(SelectorArgs),
    #[command(about = "Remove operator-configured local runtime artifacts")]
    Clean,
    #[command(about = "Check daemon control-plane readiness")]
    Doctor,
}

impl CtlCommandArgs {
    fn into_command(self, config: Option<&OperatorConfig>) -> Result<CtlCommand, String> {
        match self {
            Self::TrackAdd(args) => {
                let root_pid = args.root_pid;
                Ok(CtlCommand::TrackAdd {
                    root_pid,
                    display_name: trace_name(args.name, root_pid)?,
                    profile_name: profile_name(args.profile_name, config)?,
                    tags: args.tags.into_iter().collect(),
                })
            }
            Self::Launch(args) => {
                let root_pid = std::process::id();
                if args.argv.is_empty() {
                    return Err("launch requires a command after --".to_string());
                }
                let seccomp_config = launch_seccomp_config(config)?;
                Ok(CtlCommand::Launch {
                    display_name: trace_name(args.name, root_pid)?,
                    profile_name: profile_name(args.profile_name, config)?,
                    tags: args.tags.into_iter().collect(),
                    payload_tls_enabled: seccomp_config.payload_tls_enabled,
                    payload_tls_seccomp_syscalls: seccomp_config.payload_tls_syscalls,
                    payload_socket_enabled: seccomp_config.payload_socket_enabled,
                    payload_socket_seccomp_syscalls: seccomp_config.payload_socket_syscalls,
                    payload_socket_max_segment_bytes: seccomp_config
                        .payload_socket_max_segment_bytes,
                    process_seccomp_enabled: seccomp_config.process_enabled,
                    process_seccomp_syscalls: seccomp_config.process_syscalls,
                    seccomp_notify_reserved_listener_fd: seccomp_config.reserved_listener_fd,
                    argv: args.argv,
                })
            }
            Self::TrackRemove(args) => Ok(CtlCommand::TrackRemove {
                selector: required_selector(args)?,
            }),
            Self::ListTraces(args) => Ok(CtlCommand::ListTraces {
                selector: optional_selector(args)?,
            }),
            Self::Clean => Ok(CtlCommand::Clean {
                artifacts: CleanArtifacts::from_config(
                    config.ok_or_else(|| "missing operator config for clean".to_string())?,
                ),
            }),
            Self::Doctor => Ok(CtlCommand::Doctor),
        }
    }

    fn is_clean(&self) -> bool {
        matches!(self, Self::Clean)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LaunchSeccompConfig {
    payload_tls_enabled: bool,
    payload_tls_syscalls: Vec<PayloadTlsSeccompSyscall>,
    payload_socket_enabled: bool,
    payload_socket_syscalls: Vec<PayloadSocketSeccompSyscall>,
    payload_socket_max_segment_bytes: u32,
    process_enabled: bool,
    process_syscalls: Vec<ProcessSeccompSyscall>,
    reserved_listener_fd: u32,
}

fn launch_seccomp_config(config: Option<&OperatorConfig>) -> Result<LaunchSeccompConfig, String> {
    let config = config.ok_or_else(|| "missing operator config for launch seccomp".to_string())?;
    Ok(LaunchSeccompConfig {
        payload_tls_enabled: config.ebpf_config.payload_tls.enabled
            && config
                .ebpf_config
                .payload_tls
                .capture_backend
                .requires_seccomp_notify(),
        payload_tls_syscalls: config.ebpf_config.payload_tls.seccomp_syscalls.clone(),
        payload_socket_enabled: config.ebpf_config.payload_socket.enabled
            && config
                .ebpf_config
                .payload_socket
                .capture_backend
                .requires_seccomp_notify(),
        payload_socket_syscalls: config.ebpf_config.payload_socket.seccomp_syscalls.clone(),
        payload_socket_max_segment_bytes: config.ebpf_config.payload_socket.max_segment_bytes,
        process_enabled: config.process_seccomp.enabled,
        process_syscalls: config.process_seccomp.syscalls.clone(),
        reserved_listener_fd: config.seccomp_notify.reserved_listener_fd,
    })
}

#[derive(Clone, Debug, Args)]
struct TrackAddArgs {
    #[arg(long = "pid", value_name = "PID")]
    root_pid: u32,

    #[arg(long = "name", value_name = "NAME")]
    name: Option<String>,

    #[arg(long = "profile-name", value_name = "PROFILE")]
    profile_name: Option<String>,

    #[arg(long = "tag", action = ArgAction::Append, value_name = "TAG")]
    tags: Vec<String>,
}

#[derive(Clone, Debug, Args)]
struct LaunchArgs {
    #[arg(long = "name", value_name = "NAME")]
    name: Option<String>,

    #[arg(long = "profile-name", value_name = "PROFILE")]
    profile_name: Option<String>,

    #[arg(long = "tag", action = ArgAction::Append, value_name = "TAG")]
    tags: Vec<String>,

    #[arg(
        value_name = "COMMAND",
        required = true,
        trailing_var_arg = true,
        allow_hyphen_values = true
    )]
    argv: Vec<String>,
}

#[derive(Clone, Debug, Args)]
struct SelectorArgs {
    #[arg(long = "trace-id", value_parser = parse_trace_id, value_name = "ID")]
    trace_id: Option<TraceId>,

    #[arg(long = "root-pid", value_name = "PID")]
    root_pid: Option<u32>,

    #[arg(long = "name", value_name = "NAME")]
    name: Option<String>,

    #[arg(long = "tag-selector", value_name = "TAG")]
    tag_selector: Option<String>,
}

fn load_operator_config(
    needs_config: bool,
    config_path: &PathBuf,
) -> Result<Option<OperatorConfig>, String> {
    if !needs_config {
        return Ok(None);
    }
    OperatorConfig::load(config_path).map(Some)
}

fn default_config_path() -> PathBuf {
    PathBuf::from(DEFAULT_OPERATOR_CONFIG_PATH)
}

fn generated_request_id() -> Result<RequestId, String> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("generate request id: {error}"))?
        .as_nanos();
    let raw = u64::try_from(nanos).map_err(|error| format!("generate request id: {error}"))?;
    Ok(RequestId::new(raw))
}

fn profile_name(
    raw: Option<String>,
    config: Option<&OperatorConfig>,
) -> Result<ProfileName, String> {
    match raw {
        Some(value) if !value.is_empty() => Ok(ProfileName::new(value)),
        Some(_) => Err("invalid --profile-name: value must not be empty".to_string()),
        None => config
            .map(|config| config.capture_profile.name.clone())
            .ok_or_else(|| "missing --profile-name and operator config was not loaded".to_string()),
    }
}

fn trace_name(raw: Option<String>, root_pid: u32) -> Result<TraceName, String> {
    match raw {
        Some(value) if !value.is_empty() => Ok(TraceName::new(value)),
        Some(_) => Err("invalid --name: value must not be empty".to_string()),
        None => Ok(TraceName::new(format!("pid-{root_pid}"))),
    }
}

fn optional_selector(args: SelectorArgs) -> Result<Option<TraceSelector>, String> {
    let selectors = selector_candidates(args)?;
    match selectors.len() {
        0 => Ok(None),
        1 => Ok(selectors.into_iter().next()),
        _ => Err("selector flags are mutually exclusive".to_string()),
    }
}

fn required_selector(args: SelectorArgs) -> Result<TraceSelector, String> {
    optional_selector(args)?.ok_or_else(|| {
        "one selector flag is required: --trace-id, --root-pid, --name, or --tag-selector"
            .to_string()
    })
}

fn selector_candidates(args: SelectorArgs) -> Result<Vec<TraceSelector>, String> {
    let mut selectors = Vec::new();
    if let Some(raw) = args.trace_id {
        selectors.push(TraceSelector::TraceId(raw));
    }
    if let Some(raw) = args.root_pid {
        selectors.push(TraceSelector::RootPid(raw));
    }
    if let Some(raw) = args.name {
        if raw.is_empty() {
            return Err("invalid --name: value must not be empty".to_string());
        }
        selectors.push(TraceSelector::Name(TraceName::new(raw)));
    }
    if let Some(raw) = args.tag_selector {
        selectors.push(TraceSelector::Tag(raw));
    }
    Ok(selectors)
}

fn parse_trace_id(raw: &str) -> Result<TraceId, String> {
    let value = raw.strip_prefix("trace-").unwrap_or(raw);
    value
        .parse::<u64>()
        .map(TraceId::new)
        .map_err(|error| format!("invalid trace id: {error}"))
}
