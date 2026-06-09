//! CLI command dispatcher.

use clap::error::ErrorKind;
use tls_payload_sync::{
    RuntimeEnvConfig, RuntimeLibraryPath, launch_command_for_plan, run_with_preload, runtime_env,
    runtime_library_path, validate_native_backend_plan,
};
use tls_probe_point_finder::fast::FastProbeRequest;

use crate::cli::args::{Command, parse_args};
use crate::cli::output::{Output, write_error};
use crate::cli::reporter;
use crate::{ToolError, ToolResult};

pub(crate) fn main_from_env() -> i32 {
    match parse_args() {
        Ok(command) => match run_command(command) {
            Ok(()) => 0,
            Err(error) => {
                let _ = write_error(&error);
                1
            }
        },
        Err(error) => {
            let exit_code = cli_exit_code(&error);
            let text = error.to_string();
            let result = if exit_code == 0 {
                Output::stdout(&text)
            } else {
                Output::stderr(&text)
            };
            let _ = result;
            exit_code
        }
    }
}

fn run_command(command: Command) -> ToolResult<()> {
    match command {
        Command::Probe(args) => run_probe(args.into_config()?),
    }
}

fn run_probe(config: crate::cli::config::ProbeConfig) -> ToolResult<()> {
    let Some(program) = config.command.first() else {
        return Err(ToolError::new("probe command is empty"));
    };
    let plan = tls_probe_point_finder::fast::resolve(FastProbeRequest {
        binary: program.into(),
        arch: config.arch,
        provider: config.provider,
        source: config.source,
        match_limit: config.match_limit,
        libraries: config.libraries.clone(),
        library_search_dirs: config.library_search_dirs.clone(),
    })?;
    validate_native_backend_plan(&plan)?;
    reporter::target(&plan)?;
    let library = runtime_library_path(&RuntimeLibraryPath::Auto)?;
    let command = launch_command_for_plan(&config.command, &plan)?;
    let env_config = RuntimeEnvConfig {
        rules: config.rules.clone(),
        max_payload_bytes: config.max_payload_bytes,
        redaction: config.redaction,
        events: config.events.clone(),
        trace_id: None,
        event_socket_path: None,
    };
    let status = run_with_preload(&command, &library, runtime_env(&env_config, &plan)?)?;
    if status.success() {
        Ok(())
    } else {
        Err(ToolError::new(format!("target exited with {status}")))
    }
}

fn cli_exit_code(error: &clap::Error) -> i32 {
    match error.kind() {
        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => 0,
        _ => 2,
    }
}
