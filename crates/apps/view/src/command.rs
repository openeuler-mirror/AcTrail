//! Command-line invocation parsing for the storage viewer.

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use config_core::daemon::DEFAULT_OPERATOR_CONFIG_PATH;
use model_core::ids::TraceId;
use model_core::payload::{PayloadDirection, PayloadSegmentId};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageCommand {
    Traces,
    Summary,
    Processes,
    Events,
    Network,
    Payloads,
    Payload,
    Actions,
    Diagnostics,
    ExportJson,
    ExportOtel,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RowLimit {
    Head(usize),
    Tail(usize),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ViewInvocation {
    pub command: StorageCommand,
    pub config_path: PathBuf,
    pub storage_config_path: Option<PathBuf>,
    pub storage_path: Option<PathBuf>,
    pub output_path: Option<PathBuf>,
    pub trace_id: Option<TraceId>,
    pub row_limit: Option<RowLimit>,
    pub payload_direction: Option<PayloadDirection>,
    pub payload_segment_id: Option<PayloadSegmentId>,
    pub payload_format: Option<PayloadFormat>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PayloadFormat {
    Text,
    Hex,
}

pub fn parse_invocation(args: impl IntoIterator<Item = String>) -> Result<ViewInvocation, String> {
    let cli = ViewerCli::try_parse_from(std::iter::once("actrailviewer".to_string()).chain(args))
        .unwrap_or_else(|error| error.exit());
    cli.into_invocation()
}

#[derive(Clone, Debug, Parser)]
#[command(name = "actrailviewer", about = "Read AcTrail traces from storage")]
struct ViewerCli {
    #[arg(
        long = "config",
        global = true,
        value_name = "PATH",
        default_value = DEFAULT_OPERATOR_CONFIG_PATH
    )]
    config_path: PathBuf,

    #[arg(long = "storage-path", global = true, value_name = "PATH")]
    storage_path: Option<PathBuf>,

    #[arg(long = "storage-config", global = true, value_name = "PATH")]
    storage_config_path: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<ViewerCommandArgs>,
}

impl ViewerCli {
    fn into_invocation(self) -> Result<ViewInvocation, String> {
        Ok(ViewInvocation {
            command: self
                .command
                .as_ref()
                .map(StorageCommand::from)
                .unwrap_or(StorageCommand::Summary),
            config_path: self.config_path,
            storage_config_path: self.storage_config_path,
            storage_path: self.storage_path,
            output_path: export_output_path(&self.command),
            trace_id: trace_id_for_command(&self.command),
            row_limit: row_limit_for_command(&self.command)?,
            payload_direction: payload_direction_for_command(&self.command),
            payload_segment_id: payload_segment_id_for_command(&self.command),
            payload_format: payload_format_for_command(&self.command),
        })
    }
}

#[derive(Clone, Debug, Subcommand)]
enum ViewerCommandArgs {
    #[command(about = "List traces in storage")]
    Traces(LimitedArgs),
    #[command(about = "Show a trace summary")]
    Summary(TraceArgs),
    #[command(about = "List process memberships for a trace")]
    Processes(TraceLimitedArgs),
    #[command(about = "List events for a trace")]
    Events(TraceLimitedArgs),
    #[command(about = "List network events for a trace")]
    Network(TraceLimitedArgs),
    #[command(about = "List payload segments for a trace")]
    Payloads(PayloadListArgs),
    #[command(about = "Show one payload segment")]
    Payload(PayloadArgs),
    #[command(about = "List semantic actions for a trace")]
    Actions(TraceLimitedArgs),
    #[command(about = "List diagnostics for a trace")]
    Diagnostics(TraceLimitedArgs),
    #[command(about = "Export one trace as JSON graph")]
    ExportJson(ExportJsonArgs),
    #[command(about = "Export one trace as OpenTelemetry OTLP JSON")]
    ExportOtel(ExportJsonArgs),
}

impl From<&ViewerCommandArgs> for StorageCommand {
    fn from(command: &ViewerCommandArgs) -> Self {
        match command {
            ViewerCommandArgs::Traces(_) => Self::Traces,
            ViewerCommandArgs::Summary(_) => Self::Summary,
            ViewerCommandArgs::Processes(_) => Self::Processes,
            ViewerCommandArgs::Events(_) => Self::Events,
            ViewerCommandArgs::Network(_) => Self::Network,
            ViewerCommandArgs::Payloads(_) => Self::Payloads,
            ViewerCommandArgs::Payload(_) => Self::Payload,
            ViewerCommandArgs::Actions(_) => Self::Actions,
            ViewerCommandArgs::Diagnostics(_) => Self::Diagnostics,
            ViewerCommandArgs::ExportJson(_) => Self::ExportJson,
            ViewerCommandArgs::ExportOtel(_) => Self::ExportOtel,
        }
    }
}

#[derive(Clone, Debug, clap::Args)]
struct LimitedArgs {
    #[arg(long = "head", value_parser = parse_positive_usize, value_name = "N")]
    head: Option<usize>,

    #[arg(long = "tail", value_parser = parse_positive_usize, value_name = "N")]
    tail: Option<usize>,
}

#[derive(Clone, Debug, clap::Args)]
struct TraceArgs {
    #[arg(long = "trace-id", value_parser = parse_trace_id, value_name = "ID")]
    trace_id: Option<TraceId>,
}

#[derive(Clone, Debug, clap::Args)]
struct TraceLimitedArgs {
    #[command(flatten)]
    trace: TraceArgs,

    #[command(flatten)]
    limit: LimitedArgs,
}

#[derive(Clone, Debug, clap::Args)]
struct ExportJsonArgs {
    #[command(flatten)]
    trace: TraceArgs,

    #[arg(long = "output", value_name = "PATH")]
    output_path: Option<PathBuf>,
}

#[derive(Clone, Debug, clap::Args)]
struct PayloadListArgs {
    #[command(flatten)]
    trace: TraceArgs,

    #[command(flatten)]
    limit: LimitedArgs,

    #[arg(long = "direction", value_parser = parse_payload_direction, value_name = "DIRECTION")]
    direction: Option<PayloadDirection>,
}

#[derive(Clone, Debug, clap::Args)]
struct PayloadArgs {
    #[command(flatten)]
    trace: TraceArgs,

    #[arg(long = "segment-id", value_parser = parse_payload_segment_id, value_name = "ID")]
    segment_id: PayloadSegmentId,

    #[arg(long = "format", value_parser = parse_payload_format, default_value = "text")]
    format: PayloadFormat,
}

fn export_output_path(command: &Option<ViewerCommandArgs>) -> Option<PathBuf> {
    match command {
        Some(ViewerCommandArgs::ExportJson(args)) | Some(ViewerCommandArgs::ExportOtel(args)) => {
            args.output_path.clone()
        }
        _ => None,
    }
}

fn trace_id_for_command(command: &Option<ViewerCommandArgs>) -> Option<TraceId> {
    match command {
        Some(ViewerCommandArgs::Summary(args)) => args.trace_id,
        Some(ViewerCommandArgs::Processes(args))
        | Some(ViewerCommandArgs::Events(args))
        | Some(ViewerCommandArgs::Network(args))
        | Some(ViewerCommandArgs::Actions(args))
        | Some(ViewerCommandArgs::Diagnostics(args)) => args.trace.trace_id,
        Some(ViewerCommandArgs::Payloads(args)) => args.trace.trace_id,
        Some(ViewerCommandArgs::Payload(args)) => args.trace.trace_id,
        Some(ViewerCommandArgs::ExportJson(args)) | Some(ViewerCommandArgs::ExportOtel(args)) => {
            args.trace.trace_id
        }
        Some(ViewerCommandArgs::Traces(_)) | None => None,
    }
}

fn row_limit_for_command(command: &Option<ViewerCommandArgs>) -> Result<Option<RowLimit>, String> {
    match command {
        Some(ViewerCommandArgs::Traces(args)) => row_limit(args.head, args.tail),
        Some(ViewerCommandArgs::Processes(args))
        | Some(ViewerCommandArgs::Events(args))
        | Some(ViewerCommandArgs::Network(args))
        | Some(ViewerCommandArgs::Actions(args))
        | Some(ViewerCommandArgs::Diagnostics(args)) => row_limit(args.limit.head, args.limit.tail),
        Some(ViewerCommandArgs::Payloads(args)) => row_limit(args.limit.head, args.limit.tail),
        Some(ViewerCommandArgs::Summary(_))
        | Some(ViewerCommandArgs::ExportJson(_))
        | Some(ViewerCommandArgs::ExportOtel(_))
        | None => Ok(None),
        Some(ViewerCommandArgs::Payload(_)) => Ok(None),
    }
}

fn payload_direction_for_command(command: &Option<ViewerCommandArgs>) -> Option<PayloadDirection> {
    match command {
        Some(ViewerCommandArgs::Payloads(args)) => args.direction,
        _ => None,
    }
}

fn payload_segment_id_for_command(command: &Option<ViewerCommandArgs>) -> Option<PayloadSegmentId> {
    match command {
        Some(ViewerCommandArgs::Payload(args)) => Some(args.segment_id),
        _ => None,
    }
}

fn payload_format_for_command(command: &Option<ViewerCommandArgs>) -> Option<PayloadFormat> {
    match command {
        Some(ViewerCommandArgs::Payload(args)) => Some(args.format),
        _ => None,
    }
}

fn row_limit(head: Option<usize>, tail: Option<usize>) -> Result<Option<RowLimit>, String> {
    match (head, tail) {
        (Some(_), Some(_)) => Err("--head and --tail are mutually exclusive".to_string()),
        (Some(count), None) => Ok(Some(RowLimit::Head(count))),
        (None, Some(count)) => Ok(Some(RowLimit::Tail(count))),
        (None, None) => Ok(None),
    }
}

fn parse_trace_id(raw: &str) -> Result<TraceId, String> {
    let value = raw.strip_prefix("trace-").unwrap_or(raw);
    value
        .parse::<u64>()
        .map(TraceId::new)
        .map_err(|error| format!("invalid trace id: {error}"))
}

fn parse_positive_usize(raw: &str) -> Result<usize, String> {
    let value = raw
        .parse::<usize>()
        .map_err(|error| format!("invalid positive integer: {error}"))?;
    if value == usize::default() {
        return Err("value must be positive".to_string());
    }
    Ok(value)
}

fn parse_payload_direction(raw: &str) -> Result<PayloadDirection, String> {
    match raw {
        "outbound" => Ok(PayloadDirection::Outbound),
        "inbound" => Ok(PayloadDirection::Inbound),
        other => Err(format!("invalid payload direction {other}")),
    }
}

fn parse_payload_segment_id(raw: &str) -> Result<PayloadSegmentId, String> {
    let value = raw.strip_prefix("payload-").unwrap_or(raw);
    value
        .parse::<u64>()
        .map(PayloadSegmentId::new)
        .map_err(|error| format!("invalid payload segment id: {error}"))
}

fn parse_payload_format(raw: &str) -> Result<PayloadFormat, String> {
    match raw {
        "text" => Ok(PayloadFormat::Text),
        "hex" => Ok(PayloadFormat::Hex),
        other => Err(format!("invalid payload format {other}")),
    }
}
