use crate::plan::{ProbeSource, TlsProvider};
use crate::probe_detector::contract::detection::{
    DetectionError, DetectionEvidence, DetectionOutcome, ProbeContext,
};
use crate::probe_detector::contract::detector::{
    DetectorConfigError, ProbeDetector, ProbeDetectorConfig,
};
use crate::probe_detector::contract::identity::{DetectorId, DetectorPath};
use crate::probe_detector::contract::selection::DetectionSelector;

use super::RustlsProbeDetectorConfig;
use super::static_pattern::RustlsStaticPatternProbeDetector;
use super::symbol::RustlsSymbolProbeDetector;

pub(crate) const NAME: &str = "rustls";
pub(crate) const RESOLVER: &str = "rustls-symbol-map";
pub(crate) const RUNTIME_BUFFER_PLAINTEXT_SYMBOL: &str = "rustls_buffer_plaintext";
pub(crate) const RUNTIME_TAKE_RECEIVED_PLAINTEXT_SYMBOL: &str = "rustls_take_received_plaintext";
pub(crate) const RUNTIME_SYMBOLS: &[&str] = &[
    RUNTIME_BUFFER_PLAINTEXT_SYMBOL,
    RUNTIME_TAKE_RECEIVED_PLAINTEXT_SYMBOL,
];

pub(crate) struct RustlsProbeDetector {
    path: DetectorPath,
    selector: DetectionSelector,
    symbol: RustlsSymbolProbeDetector,
    static_pattern: RustlsStaticPatternProbeDetector,
}

impl RustlsProbeDetector {
    pub(crate) fn try_new(config: RustlsProbeDetectorConfig) -> Result<Self, DetectorConfigError> {
        config.validate()?;
        let id = DetectorId::new(NAME);
        Ok(Self {
            path: DetectorPath::root(DetectorId::new("tls")).child(id.clone()),
            selector: DetectionSelector::new(config.selection),
            symbol: RustlsSymbolProbeDetector::try_new(config.symbol)?,
            static_pattern: RustlsStaticPatternProbeDetector::try_new(config.static_pattern)?,
        })
    }
}

impl ProbeDetector for RustlsProbeDetector {
    fn path(&self) -> &DetectorPath {
        &self.path
    }

    fn detect(&self, context: &ProbeContext<'_>) -> Result<DetectionOutcome, DetectionError> {
        if context.request.requested_source == Some(ProbeSource::SharedLibrary)
            || context
                .request
                .requested_provider
                .is_some_and(|provider| provider != TlsProvider::Rustls)
        {
            return Ok(DetectionOutcome::Inapplicable(
                DetectionEvidence::new(self.path.clone(), context.target.architecture.clone())
                    .rejected("Rustls executable detector excluded by request"),
            ));
        }
        let detectors: [&dyn ProbeDetector; 2] = [&self.symbol, &self.static_pattern];
        let mut outcomes = Vec::new();
        for detector in detectors {
            let outcome = detector.detect(context)?;
            let stop = self.selector.should_stop(&outcome);
            outcomes.push(outcome);
            if stop {
                break;
            }
        }
        Ok(self.selector.select(
            DetectionEvidence::new(self.path.clone(), context.target.architecture.clone()),
            outcomes,
        ))
    }
}
