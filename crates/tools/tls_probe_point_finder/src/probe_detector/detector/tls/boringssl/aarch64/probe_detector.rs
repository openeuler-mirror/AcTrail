use std::collections::BTreeMap;

use crate::ToolResult;
use crate::elf::ElfImage;
use crate::plan::{ProbeSource, TlsProvider};
use crate::probe_detector::contract::detection::{
    DetectionError, DetectionEvidence, DetectionOutcome, ProbeContext,
};
use crate::probe_detector::contract::detector::{
    DetectorConfigError, ProbeDetector, ProbeDetectorConfig,
};
use crate::probe_detector::contract::identity::{DetectorId, DetectorPath};
use crate::probe_detector::detector::tls::boringssl::common::{
    BoringSslOutcomeFactory, StaticPatternDetection,
};
use crate::probe_detector::detector::tls::boringssl::{
    SHARED_SYMBOL_MAP_RESOLVER, STATIC_RESOLVER, SYMBOL_MAP_RESOLVER,
};

use super::Aarch64BoringSslProbeDetectorConfig;
use super::shared_library::Aarch64BoringSslSharedLibraryProbeDetector;
use super::static_pattern::Aarch64BoringSslStaticPatternProbeDetector;
use super::symbol::Aarch64BoringSslSymbolProbeDetector;

pub(crate) struct Aarch64BoringSslProbeDetector {
    path: DetectorPath,
    symbol: Aarch64BoringSslSymbolProbeDetector,
    static_pattern: Aarch64BoringSslStaticPatternProbeDetector,
    shared_library: Aarch64BoringSslSharedLibraryProbeDetector,
}

impl Aarch64BoringSslProbeDetector {
    pub(crate) fn try_new(
        config: Aarch64BoringSslProbeDetectorConfig,
    ) -> Result<Self, DetectorConfigError> {
        config.validate()?;
        Ok(Self {
            path: DetectorPath::root(DetectorId::new("tls"))
                .child(DetectorId::new("boringssl"))
                .child(DetectorId::new("aarch64")),
            symbol: Aarch64BoringSslSymbolProbeDetector::try_new(config.symbol)?,
            static_pattern: Aarch64BoringSslStaticPatternProbeDetector::try_new(
                config.static_pattern,
            )?,
            shared_library: Aarch64BoringSslSharedLibraryProbeDetector::try_new(
                config.shared_library,
            )?,
        })
    }

    pub(crate) fn resolve_symbols(
        &self,
        image: &ElfImage,
    ) -> ToolResult<Option<BTreeMap<String, u64>>> {
        self.symbol.resolve(image)
    }

    pub(crate) fn resolve_shared_library_symbols(
        &self,
        image: &ElfImage,
    ) -> ToolResult<Option<BTreeMap<String, u64>>> {
        self.shared_library.resolve(image)
    }

    pub(crate) fn detect_static(&self, image: &ElfImage) -> ToolResult<StaticPatternDetection> {
        self.static_pattern.detect(image)
    }

    pub(crate) fn register_executable_patterns(&self, image: &ElfImage) {
        self.static_pattern.register_executable_patterns(image);
    }
}

impl ProbeDetector for Aarch64BoringSslProbeDetector {
    fn path(&self) -> &DetectorPath {
        &self.path
    }

    fn detect(&self, context: &ProbeContext<'_>) -> Result<DetectionOutcome, DetectionError> {
        if context.probe.image.arch().as_str() != "aarch64" {
            return Ok(DetectionOutcome::Inapplicable(
                DetectionEvidence::new(self.path.clone(), context.target.architecture.clone())
                    .rejected("aarch64 BoringSSL detector received another architecture"),
            ));
        }
        if context.probe.source == ProbeSource::SharedLibrary {
            if let Some(symbols) = self
                .resolve_shared_library_symbols(context.probe.image)
                .map_err(|error| DetectionError::new(self.path.clone(), error.to_string()))?
            {
                return BoringSslOutcomeFactory::symbols(
                    context,
                    self.path.child(DetectorId::new("shared-library")),
                    symbols,
                    SHARED_SYMBOL_MAP_RESOLVER,
                );
            }
        } else {
            if context.request.requested_provider == Some(TlsProvider::BoringSsl) {
                if let Some(symbols) = self
                    .resolve_symbols(context.probe.image)
                    .map_err(|error| DetectionError::new(self.path.clone(), error.to_string()))?
                {
                    return BoringSslOutcomeFactory::symbols(
                        context,
                        self.path.child(DetectorId::new("symbol")),
                        symbols,
                        SYMBOL_MAP_RESOLVER,
                    );
                }
            }
            if let Ok(detection) = self.detect_static(context.probe.image) {
                return Ok(BoringSslOutcomeFactory::static_pattern(
                    context,
                    self.path.child(DetectorId::new("static-pattern")),
                    detection,
                    STATIC_RESOLVER,
                ));
            }
        }
        Ok(DetectionOutcome::NoMatch(
            DetectionEvidence::new(self.path.clone(), context.target.architecture.clone())
                .rejected("aarch64 BoringSSL detector did not match"),
        ))
    }
}
