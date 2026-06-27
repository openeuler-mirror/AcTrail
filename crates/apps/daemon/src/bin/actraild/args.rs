//! Command-line input parsing for the daemon operator binary.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use config_core::daemon::DEFAULT_OPERATOR_CONFIG_PATH;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AcTraildCommand {
    Init {
        config_path: PathBuf,
        force: bool,
    },
    Run {
        config_path: PathBuf,
    },
    Start {
        config_path: PathBuf,
    },
    Stop {
        config_path: PathBuf,
    },
    Restart {
        config_path: PathBuf,
    },
    Status {
        config_path: PathBuf,
    },
    Plugin {
        config_path: PathBuf,
        command: PluginCommand,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PluginCommand {
    Load {
        manifest_path: PathBuf,
        plugin_config_path: Option<PathBuf>,
        instance_id: String,
        host_grants: Vec<String>,
        persist: bool,
    },
    Unload {
        instance_id: String,
        persist: bool,
    },
    List,
    Status {
        instance_id: String,
    },
}

pub fn parse_args(args: impl IntoIterator<Item = String>) -> Result<AcTraildCommand, String> {
    let cli = AcTraildCli::try_parse_from(std::iter::once("actraild".to_string()).chain(args))
        .unwrap_or_else(|error| error.exit());
    cli.into_command()
}

#[derive(Clone, Debug, Parser)]
#[command(name = "actraild", about = "Run and supervise the AcTrail daemon")]
struct AcTraildCli {
    #[arg(
        long = "config",
        global = true,
        help = "Operator config path",
        value_name = "PATH"
    )]
    config_path: Option<PathBuf>,

    #[command(subcommand)]
    command: AcTraildCommandArgs,
}

impl AcTraildCli {
    fn into_command(self) -> Result<AcTraildCommand, String> {
        let explicit_config = self.config_path.is_some();
        let config_path = self.config_path.unwrap_or_else(default_config_path);
        self.command.into_command(config_path, explicit_config)
    }
}

#[derive(Clone, Debug, Subcommand)]
enum AcTraildCommandArgs {
    #[command(
        name = "init",
        visible_alias = "init-config",
        about = "Initialize the default operator config"
    )]
    Init(InitArgs),
    #[command(about = "Run the daemon in the foreground")]
    Run,
    #[command(about = "Start the daemon in the background")]
    Start,
    #[command(about = "Stop the background daemon")]
    Stop,
    #[command(about = "Restart the background daemon")]
    Restart,
    #[command(about = "Show daemon process state")]
    Status,
    #[command(about = "Manage runtime plugin instances")]
    Plugin(PluginArgs),
}

impl AcTraildCommandArgs {
    fn into_command(
        self,
        config_path: PathBuf,
        explicit_config: bool,
    ) -> Result<AcTraildCommand, String> {
        match self {
            Self::Init(args) => {
                let config_path = init_config_path(args.output_path, config_path, explicit_config)?;
                Ok(AcTraildCommand::Init {
                    config_path,
                    force: args.force,
                })
            }
            Self::Run => Ok(AcTraildCommand::Run { config_path }),
            Self::Start => Ok(AcTraildCommand::Start { config_path }),
            Self::Stop => Ok(AcTraildCommand::Stop { config_path }),
            Self::Restart => Ok(AcTraildCommand::Restart { config_path }),
            Self::Status => Ok(AcTraildCommand::Status { config_path }),
            Self::Plugin(args) => Ok(AcTraildCommand::Plugin {
                config_path,
                command: args.into_command(),
            }),
        }
    }
}

#[derive(Clone, Debug, Args)]
struct PluginArgs {
    #[command(subcommand)]
    command: PluginCommandArgs,
}

impl PluginArgs {
    fn into_command(self) -> PluginCommand {
        self.command.into_command()
    }
}

#[derive(Clone, Debug, Subcommand)]
enum PluginCommandArgs {
    #[command(about = "Load a plugin instance into the running daemon")]
    Load(PluginLoadArgs),
    #[command(about = "Unload a plugin instance from the running daemon")]
    Unload(PluginUnloadArgs),
    #[command(about = "List active plugin instances from the running daemon")]
    List,
    #[command(about = "Show one active plugin instance from the running daemon")]
    Status(PluginInstanceArgs),
}

impl PluginCommandArgs {
    fn into_command(self) -> PluginCommand {
        match self {
            Self::Load(args) => PluginCommand::Load {
                manifest_path: args.manifest_path,
                plugin_config_path: args.plugin_config_path,
                instance_id: args.instance_id,
                host_grants: args.host_grants,
                persist: args.persist,
            },
            Self::Unload(args) => PluginCommand::Unload {
                instance_id: args.instance_id,
                persist: args.persist,
            },
            Self::List => PluginCommand::List,
            Self::Status(args) => PluginCommand::Status {
                instance_id: args.instance_id,
            },
        }
    }
}

#[derive(Clone, Debug, Args)]
struct PluginLoadArgs {
    #[arg(long = "manifest", value_name = "PATH")]
    manifest_path: PathBuf,

    #[arg(long = "plugin-config", value_name = "PATH")]
    plugin_config_path: Option<PathBuf>,

    #[arg(long = "instance", value_name = "ID")]
    instance_id: String,

    #[arg(long = "grant", value_name = "CAPABILITY:VALUE")]
    host_grants: Vec<String>,

    #[arg(long = "persist")]
    persist: bool,
}

#[derive(Clone, Debug, Args)]
struct PluginUnloadArgs {
    #[arg(long = "instance", value_name = "ID")]
    instance_id: String,

    #[arg(long = "persist")]
    persist: bool,
}

#[derive(Clone, Debug, Args)]
struct PluginInstanceArgs {
    #[arg(long = "instance", value_name = "ID")]
    instance_id: String,
}

#[derive(Clone, Debug, Args)]
struct InitArgs {
    #[arg(long = "output", value_name = "PATH")]
    output_path: Option<PathBuf>,

    #[arg(short = 'f', long = "force")]
    force: bool,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_uses_default_config_path() {
        let command = parse_args(["init".to_string()]).unwrap();

        assert_eq!(
            command,
            AcTraildCommand::Init {
                config_path: PathBuf::from(DEFAULT_OPERATOR_CONFIG_PATH),
                force: false,
            }
        );
    }

    #[test]
    fn init_accepts_legacy_init_config_alias() {
        let command = parse_args(["init-config".to_string()]).unwrap();

        assert_eq!(
            command,
            AcTraildCommand::Init {
                config_path: PathBuf::from(DEFAULT_OPERATOR_CONFIG_PATH),
                force: false,
            }
        );
    }

    #[test]
    fn init_can_target_explicit_config_path() {
        let command = parse_args([
            "--config".to_string(),
            "/tmp/actrail-test.conf".to_string(),
            "init".to_string(),
        ])
        .unwrap();

        assert_eq!(
            command,
            AcTraildCommand::Init {
                config_path: PathBuf::from("/tmp/actrail-test.conf"),
                force: false,
            }
        );
    }
}
