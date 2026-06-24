//! Command-line input shapes for the control application.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use clap::{ArgAction, Args, Parser, Subcommand, ValueEnum};
use config_core::daemon::{
    DEFAULT_OPERATOR_CONFIG_PATH, OperatorConfig, PayloadSocketSeccompSyscall, PayloadTlsConfig,
    PayloadTlsSeccompSyscall, ProcessSeccompSyscall,
};
use control_contract::command::ProcessRef;
use control_contract::selector::TraceSelector;
use model_core::ids::{ProfileName, RequestId, TraceId, TraceName};

use crate::clean::CleanArtifacts;
use crate::launch::seccomp_mode::LaunchSeccompMode;
use crate::process_ref::process_ref;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CtlInvocation {
    pub socket_path: Option<PathBuf>,
    pub request_id: RequestId,
    pub command: CtlCommand,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CtlCommand {
    Init {
        config_path: PathBuf,
        force: bool,
    },
    TrackAdd {
        root: ProcessRef,
        display_name: TraceName,
        profile_name: ProfileName,
        tags: BTreeSet<String>,
    },
    Launch {
        display_name: TraceName,
        profile_name: ProfileName,
        tags: BTreeSet<String>,
        payload_tls_enabled: bool,
        payload_tls_config: PayloadTlsConfig,
        payload_tls_seccomp_syscalls: Vec<PayloadTlsSeccompSyscall>,
        payload_socket_enabled: bool,
        payload_socket_seccomp_syscalls: Vec<PayloadSocketSeccompSyscall>,
        payload_socket_max_segment_bytes: u32,
        process_seccomp_enabled: bool,
        process_seccomp_syscalls: Vec<ProcessSeccompSyscall>,
        seccomp_notify_reserved_listener_fd: u32,
        agent_invocation_commands: Vec<String>,
        seccomp_mode: LaunchSeccompMode,
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
    Probe {
        operator_config: OperatorConfig,
        json: bool,
        skip_daemon: bool,
    },
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
        let explicit_config = self.config_path.is_some();
        let config_path = self.config_path.unwrap_or_else(default_config_path);
        let init_path = self
            .command
            .init_config_path(config_path.clone(), explicit_config)?;
        let operator_config = if init_path.is_some() {
            None
        } else {
            Some(load_operator_config(&config_path)?)
        };
        let socket_path = if init_path.is_some() {
            None
        } else {
            Some(
                self.socket_path
                    .or_else(|| {
                        operator_config
                            .as_ref()
                            .map(|config| config.socket_path.clone())
                    })
                    .ok_or_else(|| {
                        "missing --socket-path and operator config was not loaded".to_string()
                    })?,
            )
        };
        let request_id = match self.request_id {
            Some(raw) => RequestId::new(raw),
            None => generated_request_id()?,
        };
        Ok(CtlInvocation {
            socket_path,
            request_id,
            command: self
                .command
                .into_command(operator_config.as_ref(), init_path)?,
        })
    }
}

#[derive(Clone, Debug, Subcommand)]
enum CtlCommandArgs {
    #[command(about = "Initialize the default operator config")]
    Init(InitArgs),
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
    #[command(about = "Probe local launch prerequisites and optional daemon readiness")]
    Probe(ProbeArgs),
}

impl CtlCommandArgs {
    fn into_command(
        self,
        config: Option<&OperatorConfig>,
        init_path: Option<PathBuf>,
    ) -> Result<CtlCommand, String> {
        match self {
            Self::Init(args) => Ok(CtlCommand::Init {
                config_path: init_path
                    .ok_or_else(|| "missing operator config path for init".to_string())?,
                force: args.force,
            }),
            Self::TrackAdd(args) => {
                let root_pid = args.root_pid;
                Ok(CtlCommand::TrackAdd {
                    root: process_ref(root_pid)?,
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
                    payload_tls_config: config
                        .ok_or_else(|| "missing operator config for launch payload".to_string())?
                        .payload_config
                        .tls
                        .clone(),
                    payload_tls_seccomp_syscalls: seccomp_config.payload_tls_syscalls,
                    payload_socket_enabled: seccomp_config.payload_socket_enabled,
                    payload_socket_seccomp_syscalls: seccomp_config.payload_socket_syscalls,
                    payload_socket_max_segment_bytes: seccomp_config
                        .payload_socket_max_segment_bytes,
                    process_seccomp_enabled: seccomp_config.process_enabled,
                    process_seccomp_syscalls: seccomp_config.process_syscalls,
                    seccomp_notify_reserved_listener_fd: seccomp_config.reserved_listener_fd,
                    agent_invocation_commands: launch_agent_commands(config),
                    seccomp_mode: args.seccomp_mode.into(),
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
            Self::Probe(args) => Ok(CtlCommand::Probe {
                operator_config: config
                    .ok_or_else(|| "missing operator config for probe".to_string())?
                    .clone(),
                json: args.json,
                skip_daemon: args.skip_daemon,
            }),
        }
    }

    fn init_config_path(
        &self,
        config_path: PathBuf,
        explicit_config: bool,
    ) -> Result<Option<PathBuf>, String> {
        match self {
            Self::Init(args) => Ok(Some(init_config_path(
                args.output_path.clone(),
                config_path,
                explicit_config,
            )?)),
            _ => Ok(None),
        }
    }
}

#[derive(Clone, Debug, Args)]
struct InitArgs {
    #[arg(long = "output", value_name = "PATH")]
    output_path: Option<PathBuf>,

    #[arg(short = 'f', long = "force")]
    force: bool,
}

fn launch_agent_commands(config: Option<&OperatorConfig>) -> Vec<String> {
    let Some(config) = config else {
        return Vec::new();
    };
    if config.agent_invocation.enabled {
        config.agent_invocation.commands.clone()
    } else {
        Vec::new()
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
        payload_tls_enabled: config.payload_config.tls.enabled
            && config
                .payload_config
                .tls
                .capture_backend
                .requires_seccomp_notify(),
        payload_tls_syscalls: config.payload_config.tls.seccomp_syscalls.clone(),
        payload_socket_enabled: config.payload_config.socket.enabled
            && config
                .payload_config
                .socket
                .capture_backend
                .requires_seccomp_notify(),
        payload_socket_syscalls: config.payload_config.socket.seccomp_syscalls.clone(),
        payload_socket_max_segment_bytes: config.payload_config.socket.max_segment_bytes,
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
struct ProbeArgs {
    #[arg(long = "json")]
    json: bool,

    #[arg(long = "skip-daemon")]
    skip_daemon: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
enum LaunchSeccompModeArg {
    #[default]
    Auto,
    Require,
    Skip,
}

impl From<LaunchSeccompModeArg> for LaunchSeccompMode {
    fn from(value: LaunchSeccompModeArg) -> Self {
        match value {
            LaunchSeccompModeArg::Auto => Self::Auto,
            LaunchSeccompModeArg::Require => Self::Require,
            LaunchSeccompModeArg::Skip => Self::Skip,
        }
    }
}

#[derive(Clone, Debug, Args)]
struct LaunchArgs {
    #[arg(long = "name", value_name = "NAME")]
    name: Option<String>,

    #[arg(long = "profile-name", value_name = "PROFILE")]
    profile_name: Option<String>,

    #[arg(long = "tag", action = ArgAction::Append, value_name = "TAG")]
    tags: Vec<String>,

    #[arg(long = "seccomp-mode", value_enum, default_value_t = LaunchSeccompModeArg::Auto)]
    seccomp_mode: LaunchSeccompModeArg,

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

fn load_operator_config(config_path: &PathBuf) -> Result<OperatorConfig, String> {
    OperatorConfig::load(config_path)
}

fn init_config_path(
    output_path: Option<PathBuf>,
    config_path: PathBuf,
    explicit_config: bool,
) -> Result<PathBuf, String> {
    match (output_path, explicit_config) {
        (Some(_), true) => Err("init accepts either --output or --config, not both".to_string()),
        (Some(path), false) => Ok(path),
        (None, true) => Ok(config_path),
        (None, false) => Ok(default_config_path()),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_uses_default_config_path_without_socket_path() {
        let invocation = parse_args(["init".to_string()]).unwrap();

        assert_eq!(invocation.socket_path, None);
        assert!(matches!(
            invocation.command,
            CtlCommand::Init {
                ref config_path,
                force: false,
            } if config_path == &PathBuf::from(DEFAULT_OPERATOR_CONFIG_PATH)
        ));
    }

    #[test]
    fn init_can_target_output_path() {
        let invocation = parse_args([
            "init".to_string(),
            "--output".to_string(),
            "/tmp/actrail-ctl-test.conf".to_string(),
        ])
        .unwrap();

        assert_eq!(invocation.socket_path, None);
        assert!(matches!(
            invocation.command,
            CtlCommand::Init {
                ref config_path,
                force: false,
            } if config_path == &PathBuf::from("/tmp/actrail-ctl-test.conf")
        ));
    }
}
