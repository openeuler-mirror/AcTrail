use crate::plan::{ProbeSource, TlsProvider};
use crate::probe_detector::contract::detection::{
    DetectionError, DetectionEvidence, DetectionOutcome, ProbeContext,
};
use crate::probe_detector::contract::detector::{
    DetectorConfigError, ProbeDetector, ProbeDetectorConfig,
};
use crate::probe_detector::contract::identity::{DetectorId, DetectorPath};
use crate::probe_detector::contract::selection::DetectionSelector;

use super::RustlsStaticPatternProbeDetectorConfig;
use super::aarch64::Aarch64RustlsProbeDetector;
use super::x86_64::X86_64RustlsProbeDetector;

pub(crate) struct RustlsStaticPatternProbeDetector {
    path: DetectorPath,
    selector: DetectionSelector,
    x86_64: X86_64RustlsProbeDetector,
    aarch64: Aarch64RustlsProbeDetector,
}

impl RustlsStaticPatternProbeDetector {
    pub(crate) fn try_new(
        config: RustlsStaticPatternProbeDetectorConfig,
    ) -> Result<Self, DetectorConfigError> {
        config.validate()?;
        let id = DetectorId::new("static-pattern");
        Ok(Self {
            path: DetectorPath::root(DetectorId::new("tls"))
                .child(DetectorId::new("rustls"))
                .child(id.clone()),
            selector: DetectionSelector::new(config.selection),
            x86_64: X86_64RustlsProbeDetector::try_new(config.x86_64)?,
            aarch64: Aarch64RustlsProbeDetector::try_new(config.aarch64)?,
        })
    }
}

impl ProbeDetector for RustlsStaticPatternProbeDetector {
    fn path(&self) -> &DetectorPath {
        &self.path
    }

    fn detect(&self, context: &ProbeContext<'_>) -> Result<DetectionOutcome, DetectionError> {
        if context.probe.source != ProbeSource::Executable
            || context
                .request
                .requested_provider
                .is_some_and(|provider| provider != TlsProvider::Rustls)
        {
            return Ok(DetectionOutcome::Inapplicable(
                DetectionEvidence::new(self.path.clone(), context.target.architecture.clone())
                    .rejected("Rustls static-pattern detector excluded by request"),
            ));
        }
        let outcomes = vec![self.x86_64.detect(context)?, self.aarch64.detect(context)?];
        Ok(self.selector.select(
            DetectionEvidence::new(self.path.clone(), context.target.architecture.clone()),
            outcomes,
        ))
    }
}
