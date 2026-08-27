//! TLS probe-point detection tool.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::path::Path;

mod args;
mod binary;
mod binary_identity;
mod detect;
mod elf;
pub mod fast;
mod pattern_cmd;
mod pattern_search;
mod plan;
mod probe_detector;
mod reporter;
mod resolve;

use args::{Command, parse_args};
pub use binary_identity::{
    BinaryIdentity, BinaryIdentityError, BinaryIdentityRegion, BinaryIdentityResolver,
    BinaryIdentityTypeCode,
};
pub use elf::{BinaryAnalysisCache, BinaryAnalysisCacheStats, ScanMode};
pub use plan::{
    AttachPoint, CaptureStrategy, PayloadDirection, ProbeBinary, ProbePoint, ProbePointPlan,
    ProbeSource, TargetIdentity, TlsProvider,
};
pub use resolve::{
    ProbeResolution, ResolveMode, resolve_plans, resolve_plans_with_analysis_cache,
    resolve_plans_with_scan,
};

pub const GO_TLS_WRITE_SYMBOL: &str = probe_detector::detector::tls::go_tls::WRITE_SYMBOL;
pub const GO_TLS_READ_SYMBOL: &str = probe_detector::detector::tls::go_tls::READ_SYMBOL;
pub const GO_RUNTIME_MEMMOVE_SYMBOL: &str =
    probe_detector::detector::tls::go_tls::RUNTIME_MEMMOVE_SYMBOL;

pub fn elf_identity(binary_path: &Path) -> ToolResult<BinaryIdentity> {
    Ok(elf::ElfImage::parse(binary_path)?.identity().clone())
}

pub fn elf_identity_from_bytes(data: &[u8]) -> ToolResult<BinaryIdentity> {
    elf::ElfImage::identity_from_data(data)
}

pub fn resolve_go_pclntab_file_offsets(
    binary_path: &Path,
    required_symbols: &[&str],
) -> ToolResult<BTreeMap<String, u64>> {
    let image = elf::ElfImage::parse(binary_path)?;
    let detector = probe_detector::detector::tls::go_tls::GoTlsProbeDetector::try_new(
        probe_detector::detector::tls::go_tls::GoTlsProbeDetectorConfig::default(),
    )?;
    let symbols = detector
        .resolve(&image, required_symbols)?
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
            let scan = args.scan;
            let plan = fast::resolve_for_consumer_with_scan(
                args.into_request(),
                fast::ProbeConsumer::PlanOnly,
                scan.into(),
            )?;
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

impl From<probe_detector::contract::detector::DetectorConfigError> for ToolError {
    fn from(error: probe_detector::contract::detector::DetectorConfigError) -> Self {
        Self::new(error.to_string())
    }
}

impl From<BinaryIdentityError> for ToolError {
    fn from(error: BinaryIdentityError) -> Self {
        Self::new(error.to_string())
    }
}
