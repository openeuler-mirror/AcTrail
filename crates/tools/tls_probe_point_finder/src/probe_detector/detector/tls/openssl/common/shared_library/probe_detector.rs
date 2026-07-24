use std::path::PathBuf;

use crate::ToolResult;
use crate::elf::{Arch, ElfImage};
use crate::probe_detector::contract::detection::{
    DetectionError, DetectionEvidence, DetectionOutcome, ProbeContext,
};
use crate::probe_detector::contract::detector::{
    DetectorConfigError, ProbeDetector, ProbeDetectorConfig,
};
use crate::probe_detector::contract::identity::{DetectorId, DetectorPath};

use super::OpenSslSharedLibraryProbeDetectorConfig;
use super::discovery::{LibrarySearch, OpenSslSharedLibraryDiscoveryProbeDetector};
use super::symbol::OpenSslSharedLibrarySymbolProbeDetector;

pub(crate) struct OpenSslSharedLibraryProbeDetector {
    path: DetectorPath,
    expected_arch: Arch,
    discovery: OpenSslSharedLibraryDiscoveryProbeDetector,
    symbol: OpenSslSharedLibrarySymbolProbeDetector,
}

impl OpenSslSharedLibraryProbeDetector {
    pub(crate) fn try_new(
        config: OpenSslSharedLibraryProbeDetectorConfig,
        expected_arch: Arch,
    ) -> Result<Self, DetectorConfigError> {
        config.validate()?;
        Ok(Self {
            path: DetectorPath::root(DetectorId::new("tls"))
                .child(DetectorId::new("openssl"))
                .child(DetectorId::new(expected_arch.as_str()))
                .child(DetectorId::new("shared-library")),
            expected_arch,
            discovery: OpenSslSharedLibraryDiscoveryProbeDetector::try_new(config.discovery)?,
            symbol: OpenSslSharedLibrarySymbolProbeDetector::try_new(config.symbol, expected_arch)?,
        })
    }

    pub(crate) fn discover(
        &self,
        image: &ElfImage,
        libraries: &[PathBuf],
        library_search_dirs: &[PathBuf],
        include_transitive: bool,
    ) -> ToolResult<LibrarySearch> {
        self.discovery
            .discover(image, libraries, library_search_dirs, include_transitive)
    }
}

impl ProbeDetector for OpenSslSharedLibraryProbeDetector {
    fn path(&self) -> &DetectorPath {
        &self.path
    }

    fn detect(&self, context: &ProbeContext<'_>) -> Result<DetectionOutcome, DetectionError> {
        if context.probe.image.arch() != self.expected_arch {
            return Ok(DetectionOutcome::Inapplicable(
                DetectionEvidence::new(self.path.clone(), context.target.architecture.clone())
                    .rejected(format!(
                        "{} OpenSSL shared-library detector received another architecture",
                        self.expected_arch.as_str()
                    )),
            ));
        }
        self.symbol.detect(context)
    }
}
