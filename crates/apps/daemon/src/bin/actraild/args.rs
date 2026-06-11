//! Command-line input parsing for the daemon operator binary.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use config_core::daemon::DEFAULT_OPERATOR_CONFIG_PATH;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AcTraildCommand {
    Init { config_path: PathBuf },
    Run { config_path: PathBuf },
    Start { config_path: PathBuf },
    Stop { config_path: PathBuf },
    Restart { config_path: PathBuf },
    Status { config_path: PathBuf },
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
                Ok(AcTraildCommand::Init { config_path })
            }
            Self::Run => Ok(AcTraildCommand::Run { config_path }),
            Self::Start => Ok(AcTraildCommand::Start { config_path }),
            Self::Stop => Ok(AcTraildCommand::Stop { config_path }),
            Self::Restart => Ok(AcTraildCommand::Restart { config_path }),
            Self::Status => Ok(AcTraildCommand::Status { config_path }),
        }
    }
}

#[derive(Clone, Debug, Args)]
struct InitArgs {
    #[arg(long = "output", value_name = "PATH")]
    output_path: Option<PathBuf>,
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
                config_path: PathBuf::from(DEFAULT_OPERATOR_CONFIG_PATH)
            }
        );
    }

    #[test]
    fn init_accepts_legacy_init_config_alias() {
        let command = parse_args(["init-config".to_string()]).unwrap();

        assert_eq!(
            command,
            AcTraildCommand::Init {
                config_path: PathBuf::from(DEFAULT_OPERATOR_CONFIG_PATH)
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
                config_path: PathBuf::from("/tmp/actrail-test.conf")
            }
        );
    }
}
