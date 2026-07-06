//! Command-line input shapes for TLS probe-point detection.

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

use crate::elf::Arch;
use crate::fast::{ArchFilter, FastProbeRequest, ProviderFilter, SourceFilter};
use crate::{ToolError, ToolResult};

const DEFAULT_MATCH_LIMIT: usize = 8;

#[derive(Clone, Debug, Eq, PartialEq, Parser)]
#[command(
    name = "tls-probe-point-finder",
    about = "Detect TLS plaintext uprobe points in ELF binaries"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Debug, Eq, PartialEq, Subcommand)]
pub(crate) enum Command {
    #[command(about = "Detect TLS provider probe points")]
    Detect(DetectArgs),
    #[command(about = "Return the first complete TLS payload probe plan")]
    Fast(FastArgs),
    #[command(about = "Extract and count bytes at a verified virtual address")]
    Pattern(PatternArgs),
}

#[derive(Clone, Debug, Eq, PartialEq, Parser)]
pub(crate) struct DetectArgs {
    /// Command name or path to the target executable or launcher.
    pub(crate) binary: PathBuf,

    /// Fail if the resolved ELF does not match this architecture.
    #[arg(long, value_enum, default_value = "auto")]
    pub(crate) arch: ArchChoice,

    /// TLS provider detector to run.
    #[arg(long, value_enum, default_value = "auto")]
    pub(crate) provider: ProviderChoice,

    /// Probe source to inspect.
    #[arg(long, value_enum, default_value = "auto")]
    pub(crate) source: SourceChoice,

    /// Extra exported function symbol to display.
    #[arg(long = "symbol", value_name = "NAME")]
    pub(crate) symbols: Vec<String>,

    /// Maximum pattern matches to print per pattern.
    #[arg(long, value_name = "N", value_parser = parse_usize, default_value_t = DEFAULT_MATCH_LIMIT)]
    pub(crate) match_limit: usize,

    /// Explicit libssl path to inspect as a shared-library candidate.
    #[arg(long = "library", value_name = "PATH")]
    pub(crate) libraries: Vec<PathBuf>,

    /// Extra directory to search for DT_NEEDED libssl entries.
    #[arg(long = "library-search-dir", value_name = "DIR")]
    pub(crate) library_search_dirs: Vec<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq, Parser)]
pub(crate) struct FastArgs {
    /// Command name or path to the target executable or launcher.
    pub(crate) binary: PathBuf,

    /// Fail if the resolved ELF does not match this architecture.
    #[arg(long, value_enum, default_value = "auto")]
    pub(crate) arch: ArchChoice,

    /// TLS provider detector to run.
    #[arg(long, value_enum, default_value = "auto")]
    pub(crate) provider: ProviderChoice,

    /// Probe source to inspect.
    #[arg(long, value_enum, default_value = "auto")]
    pub(crate) source: SourceChoice,

    /// Maximum pattern matches to inspect per pattern.
    #[arg(long, value_name = "N", value_parser = parse_usize, default_value_t = DEFAULT_MATCH_LIMIT)]
    pub(crate) match_limit: usize,

    /// Explicit libssl path to inspect as a shared-library candidate.
    #[arg(long = "library", value_name = "PATH")]
    pub(crate) libraries: Vec<PathBuf>,

    /// Extra directory to search for DT_NEEDED libssl entries.
    #[arg(long = "library-search-dir", value_name = "DIR")]
    pub(crate) library_search_dirs: Vec<PathBuf>,
}

impl FastArgs {
    pub(crate) fn into_request(self) -> FastProbeRequest {
        FastProbeRequest {
            binary: self.binary,
            arch: self.arch.into(),
            provider: self.provider.into(),
            source: self.source.into(),
            match_limit: self.match_limit,
            libraries: self.libraries,
            library_search_dirs: self.library_search_dirs,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Parser)]
pub(crate) struct PatternArgs {
    /// Command name or path to the target executable or launcher.
    pub(crate) binary: PathBuf,

    /// Fail if the resolved ELF does not match this architecture.
    #[arg(long, value_enum, default_value = "auto")]
    pub(crate) arch: ArchChoice,

    /// Virtual address whose bytes should be extracted.
    #[arg(long, value_name = "ADDR", value_parser = parse_u64)]
    pub(crate) address: u64,

    /// Number of bytes to extract. Hex values such as 0x20 are accepted.
    #[arg(long, value_name = "N", value_parser = parse_usize)]
    pub(crate) length: usize,

    /// Maximum matching locations to print.
    #[arg(long, value_name = "N", value_parser = parse_usize, default_value_t = DEFAULT_MATCH_LIMIT)]
    pub(crate) match_limit: usize,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum ArchChoice {
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

impl ArchChoice {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Aarch64 => "aarch64",
            Self::X86_64 => "x86_64",
        }
    }
}

pub(crate) fn require_arch(
    actual: Arch,
    requested: ArchChoice,
    path: &std::path::Path,
) -> ToolResult<()> {
    let matches = match requested {
        ArchChoice::Auto => true,
        ArchChoice::Aarch64 => actual == Arch::Aarch64,
        ArchChoice::X86_64 => actual == Arch::X86_64,
    };
    if matches {
        Ok(())
    } else {
        Err(ToolError::new(format!(
            "{} is {}, not {}",
            path.display(),
            actual.as_str(),
            requested.label()
        )))
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum ProviderChoice {
    Auto,
    #[value(name = "openssl")]
    OpenSsl,
    #[value(name = "boringssl")]
    BoringSsl,
    Rustls,
    Go,
    #[value(name = "gnutls")]
    GnuTls,
    Nss,
}

impl From<ProviderChoice> for ProviderFilter {
    fn from(choice: ProviderChoice) -> Self {
        match choice {
            ProviderChoice::Auto => Self::Auto,
            ProviderChoice::OpenSsl => Self::OpenSsl,
            ProviderChoice::BoringSsl => Self::BoringSsl,
            ProviderChoice::Rustls => Self::Rustls,
            ProviderChoice::Go => Self::Go,
            ProviderChoice::GnuTls => Self::GnuTls,
            ProviderChoice::Nss => Self::Nss,
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum SourceChoice {
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

pub(crate) fn parse_args() -> Command {
    Cli::parse().command
}

fn parse_usize(value: &str) -> Result<usize, String> {
    usize::try_from(parse_u64(value)?).map_err(|_| format!("integer value is too large: {value}"))
}

fn parse_u64(value: &str) -> Result<u64, String> {
    if let Some(hex) = value.strip_prefix("0x") {
        u64::from_str_radix(hex, 16)
    } else {
        value.parse()
    }
    .map_err(|error| format!("invalid integer {value}: {error}"))
}
