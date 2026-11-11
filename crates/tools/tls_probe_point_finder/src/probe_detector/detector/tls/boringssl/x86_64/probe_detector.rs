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
use crate::probe_detector::contract::selection::DetectionSelector;
use crate::probe_detector::detector::tls::boringssl::common::{
    BoringSslOutcomeFactory, StaticPatternDetection,
};
use crate::probe_detector::detector::tls::boringssl::{
    SHARED_SYMBOL_MAP_RESOLVER, STATIC_RESOLVER, SYMBOL_MAP_RESOLVER,
};

use super::X86_64BoringSslProbeDetectorConfig;
use super::read_write_pair_19_40::ReadWritePair1940ProbeDetector;
use super::shared_library::X86_64BoringSslSharedLibraryProbeDetector;
use super::static_pattern::X86_64BoringSslStaticPatternProbeDetector;
use super::symbol::X86_64BoringSslSymbolProbeDetector;

pub(crate) struct X86_64BoringSslProbeDetector {
    path: DetectorPath,
    symbol: X86_64BoringSslSymbolProbeDetector,
    static_pattern: X86_64BoringSslStaticPatternProbeDetector,
    read_write_pair_19_40: ReadWritePair1940ProbeDetector,
    static_selector: DetectionSelector,
    shared_library: X86_64BoringSslSharedLibraryProbeDetector,
}

impl X86_64BoringSslProbeDetector {
    pub(crate) fn try_new(
        config: X86_64BoringSslProbeDetectorConfig,
    ) -> Result<Self, DetectorConfigError> {
        config.validate()?;
        Ok(Self {
            path: DetectorPath::root(DetectorId::new("tls"))
                .child(DetectorId::new("boringssl"))
                .child(DetectorId::new("x86_64")),
            symbol: X86_64BoringSslSymbolProbeDetector::try_new(config.symbol)?,
            static_pattern: X86_64BoringSslStaticPatternProbeDetector::try_new(
                config.static_pattern,
            )?,
            read_write_pair_19_40: ReadWritePair1940ProbeDetector::try_new(
                config.read_write_pair_19_40,
            )?,
            static_selector: DetectionSelector::new(config.static_selection),
            shared_library: X86_64BoringSslSharedLibraryProbeDetector::try_new(
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

    pub(crate) fn register_executable_patterns(&self, image: &ElfImage) {
        self.static_pattern.register_executable_patterns(image);
        self.read_write_pair_19_40
            .register_executable_patterns(image);
    }

    fn static_outcome(
        &self,
        context: &ProbeContext<'_>,
        path: DetectorPath,
        detection: ToolResult<Option<StaticPatternDetection>>,
    ) -> DetectionOutcome {
        match detection {
            Ok(Some(detection)) => {
                BoringSslOutcomeFactory::static_pattern(context, path, detection, STATIC_RESOLVER)
            }
            Ok(None) => DetectionOutcome::Inapplicable(
                DetectionEvidence::new(path, context.target.architecture.clone())
                    .rejected("candidate excluded by verified GNU BuildID"),
            ),
            Err(error) => DetectionOutcome::NoMatch(
                DetectionEvidence::new(path, context.target.architecture.clone())
                    .rejected(error.to_string()),
            ),
        }
    }
}

impl ProbeDetector for X86_64BoringSslProbeDetector {
    fn path(&self) -> &DetectorPath {
        &self.path
    }

    fn detect(&self, context: &ProbeContext<'_>) -> Result<DetectionOutcome, DetectionError> {
        if context.probe.image.arch().as_str() != "x86_64" {
            return Ok(DetectionOutcome::Inapplicable(
                DetectionEvidence::new(self.path.clone(), context.target.architecture.clone())
                    .rejected("x86_64 BoringSSL detector received another architecture"),
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
            return Ok(self.static_selector.select(
                DetectionEvidence::new(self.path.clone(), context.target.architecture.clone()),
                vec![
                    self.static_outcome(
                        context,
                        self.path.child(DetectorId::new("static-pattern")),
                        self.static_pattern.detect(context.probe.image).map(Some),
                    ),
                    self.static_outcome(
                        context,
                        self.path.child(DetectorId::new("read-write-pair-19-40")),
                        self.read_write_pair_19_40.detect(context.probe.image),
                    ),
                ],
            ));
        }
        Ok(DetectionOutcome::NoMatch(
            DetectionEvidence::new(self.path.clone(), context.target.architecture.clone())
                .rejected("x86_64 BoringSSL detector did not match"),
        ))
    }
}
