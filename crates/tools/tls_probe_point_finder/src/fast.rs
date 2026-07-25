//! Fast probe-point resolution for payload capture startup.

use std::path::PathBuf;

use crate::binary::resolve_entry_elf;
use crate::elf::{Arch, ElfImage};
use crate::plan::{ProbePointPlan, ProbeSource, TargetIdentity, TlsProvider};
use crate::probe_detector::contract::detection::{
    DetectionOutcome, DetectionRequest, ProbeContext,
};
use crate::probe_detector::contract::detector::ProbeDetector;
use crate::probe_detector::detector::tls::{TlsProbeDetector, TlsProbeDetectorConfig};
use crate::{ToolError, ToolResult};

pub use crate::probe_detector::contract::detection::ProbeConsumer;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FastProbeRequest {
    pub binary: PathBuf,
    pub arch: ArchFilter,
    pub provider: ProviderFilter,
    pub source: SourceFilter,
    pub match_limit: usize,
    pub libraries: Vec<PathBuf>,
    pub library_search_dirs: Vec<PathBuf>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArchFilter {
    Auto,
    Aarch64,
    X86_64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderFilter {
    Auto,
    OpenSsl,
    BoringSsl,
    Rustls,
    Go,
    GnuTls,
    Nss,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceFilter {
    Auto,
    Executable,
    SharedLibrary,
}

pub fn resolve(request: FastProbeRequest) -> ToolResult<ProbePointPlan> {
    resolve_for_consumer(request, ProbeConsumer::PlanOnly)
}

pub fn resolve_for_consumer(
    request: FastProbeRequest,
    consumer: ProbeConsumer,
) -> ToolResult<ProbePointPlan> {
    let binary = resolve_entry_elf(&request.binary)?;
    let image = ElfImage::parse(&binary)?;
    require_arch(image.arch(), request.arch, image.path())?;
    let target = TargetIdentity {
        binary: image.path().to_path_buf(),
        architecture: image.arch().as_str().to_string(),
        identity: image.identity().clone(),
    };
    let detection_request = DetectionRequest {
        requested_provider: request.provider.requested_provider(),
        requested_source: request.source.requested_source(),
        libraries: request.libraries,
        library_search_dirs: request.library_search_dirs,
        consumer,
    };
    let context = ProbeContext::executable(&target, &image, &detection_request);
    let detector = TlsProbeDetector::try_new(TlsProbeDetectorConfig::with_match_limit(
        request.match_limit,
    ))?;
    match detector
        .detect(&context)
        .map_err(|error| ToolError::new(error.to_string()))?
    {
        DetectionOutcome::Matched(candidate) => Ok(candidate.into_plan()),
        DetectionOutcome::Ambiguous(ambiguous) => Err(ToolError::new(format!(
            "ambiguous TLS probe detection: {}",
            ambiguous
                .candidates
                .iter()
                .map(|candidate| candidate.detector_path.display())
                .collect::<Vec<_>>()
                .join(", ")
        ))),
        DetectionOutcome::Inapplicable(_)
        | DetectionOutcome::NoMatch(_)
        | DetectionOutcome::Collected(_) => Err(ToolError::new(
            "no supported TLS payload probe points found",
        )),
    }
}

fn require_arch(actual: Arch, requested: ArchFilter, path: &std::path::Path) -> ToolResult<()> {
    let matches = match requested {
        ArchFilter::Auto => true,
        ArchFilter::Aarch64 => actual == Arch::Aarch64,
        ArchFilter::X86_64 => actual == Arch::X86_64,
    };
    if matches {
        Ok(())
    } else {
        Err(ToolError::new(format!(
            "{} is {}, not {}",
            path.display(),
            actual.as_str(),
            requested.as_str()
        )))
    }
}

impl ArchFilter {
    fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Aarch64 => "aarch64",
            Self::X86_64 => "x86_64",
        }
    }
}

impl ProviderFilter {
    fn requested_provider(self) -> Option<TlsProvider> {
        match self {
            Self::Auto => None,
            Self::OpenSsl => Some(TlsProvider::OpenSsl),
            Self::BoringSsl => Some(TlsProvider::BoringSsl),
            Self::Rustls => Some(TlsProvider::Rustls),
            Self::Go => Some(TlsProvider::Go),
            Self::GnuTls => Some(TlsProvider::GnuTls),
            Self::Nss => Some(TlsProvider::Nss),
        }
    }
}

impl SourceFilter {
    fn requested_source(self) -> Option<ProbeSource> {
        match self {
            Self::Auto => None,
            Self::Executable => Some(ProbeSource::Executable),
            Self::SharedLibrary => Some(ProbeSource::SharedLibrary),
        }
    }
}
