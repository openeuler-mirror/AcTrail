//! `actrailctl checkpoint` subcommands, the `--checkpoint` launch file and reply formatting.

use std::path::{Path, PathBuf};

use clap::Subcommand;
use control_contract::checkpoint::{
    CheckpointAction, CheckpointFilterMode, CheckpointReply, CheckpointView,
    LaunchCheckpointConfig, LaunchCheckpointScope, OperationView,
};
use model_core::ids::TraceId;
use serde::Deserialize;

#[derive(Clone, Debug, Subcommand)]
pub(crate) enum CheckpointArgs {
    #[command(about = "Save every scope of a running trace now")]
    Save {
        #[arg(value_parser = parse_trace_id)]
        trace_id: TraceId,
    },
    #[command(
        about = "Restore a trace to one of its checkpoints; a running trace is restored at its next LLM request"
    )]
    Restore {
        #[arg(value_parser = parse_trace_id)]
        trace_id: TraceId,
        #[arg(value_parser = parse_checkpoint_id)]
        checkpoint_id: u64,
    },
    #[command(about = "Delete a checkpoint and release its snapshot")]
    Delete {
        #[arg(value_parser = parse_trace_id)]
        trace_id: TraceId,
        #[arg(value_parser = parse_checkpoint_id)]
        checkpoint_id: u64,
    },
    #[command(
        about = "Give up the checkpoints of a trace: delete the remaining ones and stop saving it"
    )]
    Disconnect {
        #[arg(value_parser = parse_trace_id)]
        trace_id: TraceId,
    },
    #[command(about = "Start a restore that waits for an LLM request now")]
    Force {
        #[arg(value_parser = parse_operation_id)]
        operation_id: u64,
    },
    #[command(about = "Continue the processes a failed restore left stopped")]
    Resume {
        #[arg(value_parser = parse_trace_id)]
        trace_id: TraceId,
    },
    #[command(
        about = "List the checkpoints of a trace, deleted ones included: all, the newest N, or those created in a time range"
    )]
    List {
        #[arg(value_parser = parse_trace_id)]
        trace_id: TraceId,
        #[arg(
            long,
            conflicts_with_all = ["since_unix_nanos", "until_unix_nanos"],
            help = "Only the newest N checkpoints"
        )]
        last: Option<u32>,
        #[arg(
            long = "since",
            value_parser = parse_unix_secs,
            help = "Only checkpoints created at or after this Unix time in seconds"
        )]
        since_unix_nanos: Option<u64>,
        #[arg(
            long = "until",
            value_parser = parse_unix_secs,
            help = "Only checkpoints created before this Unix time in seconds"
        )]
        until_unix_nanos: Option<u64>,
    },
    #[command(about = "List the saves and restores of a trace")]
    Operations {
        #[arg(value_parser = parse_trace_id)]
        trace_id: TraceId,
    },
    #[command(about = "Show the progress and result of one operation")]
    Operation {
        #[arg(value_parser = parse_operation_id)]
        operation_id: u64,
    },
}

impl CheckpointArgs {
    pub(crate) fn into_action(self) -> CheckpointAction {
        match self {
            Self::Save { trace_id } => CheckpointAction::Save { trace_id },
            Self::Restore {
                trace_id,
                checkpoint_id,
            } => CheckpointAction::Restore {
                trace_id,
                checkpoint_id,
            },
            Self::Delete {
                trace_id,
                checkpoint_id,
            } => CheckpointAction::Delete {
                trace_id,
                checkpoint_id,
            },
            Self::Disconnect { trace_id } => CheckpointAction::Disconnect { trace_id },
            Self::Force { operation_id } => CheckpointAction::Force { operation_id },
            Self::Resume { trace_id } => CheckpointAction::Resume { trace_id },
            Self::List {
                trace_id,
                last,
                since_unix_nanos,
                until_unix_nanos,
            } => CheckpointAction::List {
                trace_id,
                filter_mode: match (last, since_unix_nanos, until_unix_nanos) {
                    (Some(count), _, _) => CheckpointFilterMode::Last(count),
                    (None, None, None) => CheckpointFilterMode::All,
                    (None, since_unix_nanos, until_unix_nanos) => CheckpointFilterMode::Created {
                        since_unix_nanos,
                        until_unix_nanos,
                    },
                },
            },
            Self::Operations { trace_id } => CheckpointAction::Operations { trace_id },
            Self::Operation { operation_id } => CheckpointAction::Operation { operation_id },
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LaunchFile {
    scopes: Vec<ScopeEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScopeEntry {
    id: String,
    root: PathBuf,
    #[serde(default)]
    exclude: Vec<String>,
    backend_instance: String,
}

/// Reads the `--checkpoint` file. Roots are resolved here, in the Agent's filesystem view, so
/// the daemon and backends receive the directories the Agent actually uses.
pub(crate) fn load_launch_config(path: &Path) -> Result<LaunchCheckpointConfig, String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|error| format!("read checkpoint config {}: {error}", path.display()))?;
    let file: LaunchFile = toml::from_str(&raw)
        .map_err(|error| format!("parse checkpoint config {}: {error}", path.display()))?;
    let scopes = file
        .scopes
        .into_iter()
        .map(|scope| {
            if !scope.root.is_absolute() {
                return Err(format!(
                    "checkpoint scope {} root {} must be absolute",
                    scope.id,
                    scope.root.display()
                ));
            }
            let root = scope.root.canonicalize().map_err(|error| {
                format!(
                    "checkpoint scope {} root {}: {error}",
                    scope.id,
                    scope.root.display()
                )
            })?;
            Ok(LaunchCheckpointScope {
                id: scope.id,
                root: root.to_string_lossy().into_owned(),
                excludes: scope.exclude,
                backend_instance: scope.backend_instance,
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(LaunchCheckpointConfig { scopes })
}

pub(crate) fn format_reply(reply: &CheckpointReply) -> String {
    match reply {
        CheckpointReply::OperationAccepted { operation_id } => {
            format!("checkpoint operation-{operation_id} accepted")
        }
        CheckpointReply::Accepted => "checkpoint request accepted".to_string(),
        CheckpointReply::Status { held, busy } => format!("checkpoint held={held} busy={busy}"),
        CheckpointReply::Checkpoints(checkpoints) if checkpoints.is_empty() => {
            "no checkpoints".to_string()
        }
        CheckpointReply::Checkpoints(checkpoints) => checkpoints
            .iter()
            .map(format_checkpoint)
            .collect::<Vec<_>>()
            .join("\n"),
        CheckpointReply::Operations(operations) if operations.is_empty() => {
            "no checkpoint operations".to_string()
        }
        CheckpointReply::Operations(operations) => operations
            .iter()
            .map(operation_line)
            .collect::<Vec<_>>()
            .join("\n"),
        CheckpointReply::Operation(operation) => {
            let mut lines = vec![operation_line(operation)];
            if let Some(error) = &operation.error {
                lines.push(format!("  error {error}"));
            }
            if !operation.invalidated_checkpoints.is_empty() {
                let invalidated: Vec<String> = operation
                    .invalidated_checkpoints
                    .iter()
                    .map(|checkpoint| format!("checkpoint-{checkpoint}"))
                    .collect();
                lines.push(format!("  invalidated {}", invalidated.join(",")));
            }
            lines.join("\n")
        }
    }
}

fn format_checkpoint(checkpoint: &CheckpointView) -> String {
    let state = if checkpoint.restorable {
        "restorable".to_string()
    } else if let Some(release) = &checkpoint.release {
        format!("deleted release={release}")
    } else if let Some(operation) = checkpoint.invalidated_by {
        format!("invalidated_by=operation-{operation}")
    } else {
        "not_restorable".to_string()
    };
    let mut line = format!(
        "checkpoint-{} created_unix_secs={} from=operation-{} {state}",
        checkpoint.checkpoint_id,
        checkpoint.created_at_unix_nanos / 1_000_000_000,
        checkpoint.operation_id,
    );
    if let Some(error) = &checkpoint.release_error {
        line.push_str(&format!(" release_error={error}"));
    }
    line
}

fn operation_line(operation: &OperationView) -> String {
    let mut line = format!(
        "operation-{} trace-{} {} trigger={} phase={}",
        operation.operation_id,
        operation.trace_id,
        operation.kind,
        operation.trigger,
        operation.phase
    );
    if let Some(outcome) = &operation.outcome {
        line.push_str(&format!(" outcome={outcome}"));
    }
    if let Some(target) = operation.target_checkpoint {
        line.push_str(&format!(" target=checkpoint-{target}"));
    }
    if let Some(created) = operation.created_checkpoint {
        line.push_str(&format!(" created=checkpoint-{created}"));
    }
    line
}

fn parse_trace_id(raw: &str) -> Result<TraceId, String> {
    parse_id(raw, "trace-").map(TraceId::new)
}

fn parse_checkpoint_id(raw: &str) -> Result<u64, String> {
    parse_id(raw, "checkpoint-")
}

fn parse_operation_id(raw: &str) -> Result<u64, String> {
    parse_id(raw, "operation-")
}

/// Takes the seconds `list` prints as `created_unix_secs` and returns nanoseconds.
fn parse_unix_secs(raw: &str) -> Result<u64, String> {
    raw.parse::<u64>()
        .ok()
        .and_then(|secs| secs.checked_mul(1_000_000_000))
        .ok_or_else(|| format!("{raw} is not a Unix time in seconds"))
}

/// Accepts both `7` and the displayed form `checkpoint-7`.
fn parse_id(raw: &str, prefix: &str) -> Result<u64, String> {
    raw.strip_prefix(prefix)
        .unwrap_or(raw)
        .parse()
        .map_err(|error| format!("invalid id {raw}: {error}"))
}
