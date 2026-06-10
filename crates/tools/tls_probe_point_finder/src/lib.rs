//! TLS probe-point detection tool.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::path::Path;

mod args;
mod binary;
mod detect;
mod elf;
pub mod fast;
mod pattern_cmd;
mod plan;
mod providers;
mod reporter;

use args::{Command, parse_args};
pub use plan::{
    AttachPoint, CaptureStrategy, PayloadDirection, ProbeBinary, ProbePoint, ProbePointPlan,
    ProbeSource, TargetIdentity, TlsProvider,
};

pub const GO_TLS_WRITE_SYMBOL: &str = providers::go_tls::WRITE_SYMBOL;
pub const GO_TLS_READ_SYMBOL: &str = providers::go_tls::READ_SYMBOL;
pub const GO_RUNTIME_MEMMOVE_SYMBOL: &str = providers::go_tls::RUNTIME_MEMMOVE_SYMBOL;

pub fn resolve_go_pclntab_file_offsets(
    binary_path: &Path,
    required_symbols: &[&str],
) -> ToolResult<BTreeMap<String, u64>> {
    let image = elf::ElfImage::parse(binary_path)?;
    let symbols = providers::go_tls::resolve_pclntab_symbols(&image, required_symbols)?
        .ok_or_else(|| ToolError::new("missing Go crypto/tls pclntab symbols"))?;
    required_symbols
        .iter()
        .map(|symbol| {
            let virtual_address = symbols.get(*symbol).copied().ok_or_else(|| {
                ToolError::new(format!("missing Go crypto/tls pclntab symbol {symbol}"))
            })?;
            let file_offset = image.file_offset_for_virtual_address(virtual_address)?;
            Ok(((*symbol).to_string(), file_offset))
        })
        .collect()
}

pub fn run_from_env() -> ToolResult<()> {
    match parse_args() {
        Command::Detect(args) => {
            let report = detect::run(args)?;
            reporter::print_detect_report(&report)?;
            if report.success() {
                Ok(())
            } else {
                Err(ToolError::new(report.failure_message()))
            }
        }
        Command::Fast(args) => {
            let plan = fast::resolve(args.into_request())?;
            reporter::print_fast_probe_plan(&plan)
        }
        Command::Pattern(args) => {
            let report = pattern_cmd::run(args)?;
            reporter::print_pattern_report(&report)
        }
    }
}

pub type ToolResult<T> = Result<T, ToolError>;

#[derive(Debug)]
pub struct ToolError {
    message: String,
}

impl ToolError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl Display for ToolError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for ToolError {}

impl From<std::io::Error> for ToolError {
    fn from(error: std::io::Error) -> Self {
        Self::new(error.to_string())
    }
}
