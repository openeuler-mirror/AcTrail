//! Command-line input shapes.

use std::ffi::OsString;
use std::path::PathBuf;
use std::str::FromStr;

use clap::{Parser, Subcommand, ValueEnum};
use tls_payload_core::{PayloadDirection, RewriteRule};
use tls_probe_point_finder::fast::{ArchFilter, ProviderFilter, SourceFilter};

use crate::cli::config::{
    DEFAULT_MATCH_LIMIT, DEFAULT_MAX_PAYLOAD_BYTES, ProbeConfig, RedactionMode, ReportEvent,
    event_filter_from_events,
};
use crate::{ToolError, ToolResult};

#[derive(Clone, Debug, Parser)]
#[command(
    name = "tls-payload-probe-sync",
    about = "Synchronously rewrite TLS plaintext with native hooks"
)]
pub(crate) struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Debug, Subcommand)]
pub(crate) enum Command {
    #[command(about = "Launch a command with synchronous TLS plaintext rewrite hooks")]
    Probe(ProbeArgs),
}

#[derive(Clone, Debug, Parser)]
pub(crate) struct ProbeArgs {
    /// Fail if the resolved ELF does not match this architecture.
    #[arg(long, value_enum, default_value = "auto")]
    arch: ArchChoice,

    /// TLS provider to use for fast probe-point resolution.
    #[arg(long, value_enum, default_value = "auto")]
    provider: ProviderChoice,

    /// Probe source to inspect.
    #[arg(long, value_enum, default_value = "auto")]
    source: SourceChoice,

    /// Maximum static pattern matches to inspect per pattern.
    #[arg(long, value_name = "N", value_parser = parse_usize, default_value_t = DEFAULT_MATCH_LIMIT)]
    match_limit: usize,

    /// Maximum bytes eligible for a synchronous payload decision.
    #[arg(long, value_name = "N", value_parser = parse_usize, default_value_t = DEFAULT_MAX_PAYLOAD_BYTES)]
    max_payload_bytes: usize,

    /// Equal-byte UTF-8 rewrite rule: direction:from=to.
    #[arg(long = "replace-text", value_name = "RULE")]
    replace_text: Vec<String>,

    /// Equal-byte hex rewrite rule: direction:from_hex=to_hex.
    #[arg(long = "replace-hex", value_name = "RULE")]
    replace_hex: Vec<String>,

    /// Payload preview redaction mode.
    #[arg(long, value_enum, default_value = "redact")]
    redaction: RedactionChoice,

    /// Event groups to print. Omit this option to print every group.
    #[arg(long = "events", value_enum, value_delimiter = ',', num_args = 1..)]
    events: Vec<EventChoice>,

    /// Explicit libssl path to inspect as a shared-library candidate.
    #[arg(long = "library", value_name = "PATH")]
    libraries: Vec<PathBuf>,

    /// Extra directory to search for DT_NEEDED libssl entries.
    #[arg(long = "library-search-dir", value_name = "DIR")]
    library_search_dirs: Vec<PathBuf>,

    /// Target command. The first item is the agent program.
    #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
    command: Vec<OsString>,
}

impl ProbeArgs {
    pub(crate) fn into_config(self) -> ToolResult<ProbeConfig> {
        if self.command.is_empty() {
            return Err(ToolError::new("probe command is empty"));
        }
        if self.max_payload_bytes == 0 {
            return Err(ToolError::new("max_payload_bytes must be positive"));
        }
        let rules = parse_rules(&self.replace_text, &self.replace_hex)?;
        Ok(ProbeConfig {
            command: self.command,
            arch: self.arch.into(),
            provider: self.provider.into(),
            source: self.source.into(),
            match_limit: self.match_limit,
            libraries: self.libraries,
            library_search_dirs: self.library_search_dirs,
            rules,
            max_payload_bytes: self.max_payload_bytes,
            redaction: self.redaction.into(),
            events: event_filter_from_events(
                &self
                    .events
                    .iter()
                    .map(|event| (*event).into())
                    .collect::<Vec<_>>(),
            ),
        })
    }
}

pub(crate) fn parse_args() -> Result<Command, clap::Error> {
    Ok(Cli::try_parse()?.command)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum ArchChoice {
    Auto,
    Aarch64,
    #[value(name = "x86_64")]
    X86_64,
}

impl From<ArchChoice> for ArchFilter {
    fn from(choice: ArchChoice) -> Self {
        match choice {
            ArchChoice::Auto => Self::Auto,
            ArchChoice::Aarch64 => Self::Aarch64,
            ArchChoice::X86_64 => Self::X86_64,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum ProviderChoice {
    Auto,
    #[value(name = "openssl")]
    OpenSsl,
    #[value(name = "boringssl")]
    BoringSsl,
    Rustls,
}

impl From<ProviderChoice> for ProviderFilter {
    fn from(choice: ProviderChoice) -> Self {
        match choice {
            ProviderChoice::Auto => Self::Auto,
            ProviderChoice::OpenSsl => Self::OpenSsl,
            ProviderChoice::BoringSsl => Self::BoringSsl,
            ProviderChoice::Rustls => Self::Rustls,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum SourceChoice {
    Auto,
    Executable,
    #[value(name = "shared-library")]
    SharedLibrary,
}

impl From<SourceChoice> for SourceFilter {
    fn from(choice: SourceChoice) -> Self {
        match choice {
            SourceChoice::Auto => Self::Auto,
            SourceChoice::Executable => Self::Executable,
            SourceChoice::SharedLibrary => Self::SharedLibrary,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum RedactionChoice {
    Redact,
    None,
}

impl From<RedactionChoice> for RedactionMode {
    fn from(choice: RedactionChoice) -> Self {
        match choice {
            RedactionChoice::Redact => Self::Redact,
            RedactionChoice::None => Self::None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum EventChoice {
    Target,
    Payload,
    Decision,
}

impl From<EventChoice> for ReportEvent {
    fn from(choice: EventChoice) -> Self {
        match choice {
            EventChoice::Target => Self::Target,
            EventChoice::Payload => Self::Payload,
            EventChoice::Decision => Self::Decision,
        }
    }
}

fn parse_rules(text_rules: &[String], hex_rules: &[String]) -> ToolResult<Vec<RewriteRule>> {
    let mut rules = Vec::new();
    for value in text_rules {
        let (direction, from, to) = parse_rule_parts(value)?;
        rules.push(RewriteRule::new(
            direction,
            from.as_bytes().to_vec(),
            to.as_bytes().to_vec(),
            format!("text:{value}"),
        )?);
    }
    for value in hex_rules {
        let (direction, from, to) = parse_rule_parts(value)?;
        rules.push(RewriteRule::new(
            direction,
            decode_hex(from)?,
            decode_hex(to)?,
            format!("hex:{value}"),
        )?);
    }
    Ok(rules)
}

fn parse_rule_parts(value: &str) -> ToolResult<(PayloadDirection, &str, &str)> {
    let (direction, rest) = value
        .split_once(':')
        .ok_or_else(|| ToolError::new(format!("rewrite rule missing direction: {value}")))?;
    let (from, to) = rest
        .split_once('=')
        .ok_or_else(|| ToolError::new(format!("rewrite rule missing '=': {value}")))?;
    Ok((PayloadDirection::from_str(direction)?, from, to))
}

fn decode_hex(value: &str) -> ToolResult<Vec<u8>> {
    if value.len() % 2 != 0 {
        return Err(ToolError::new(format!(
            "hex rule value must have even length: {value}"
        )));
    }
    let mut bytes = Vec::with_capacity(value.len() / 2);
    for chunk in value.as_bytes().chunks_exact(2) {
        let text = std::str::from_utf8(chunk)
            .map_err(|error| ToolError::new(format!("hex rule is not utf8: {error}")))?;
        let byte = u8::from_str_radix(text, 16)
            .map_err(|error| ToolError::new(format!("invalid hex byte {text}: {error}")))?;
        bytes.push(byte);
    }
    Ok(bytes)
}

fn parse_usize(value: &str) -> Result<usize, String> {
    value.parse().map_err(|error| format!("{error}"))
}
