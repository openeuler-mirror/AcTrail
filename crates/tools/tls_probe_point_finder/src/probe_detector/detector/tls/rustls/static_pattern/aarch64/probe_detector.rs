use crate::elf::ElfImage;
use crate::probe_detector::contract::detection::{
    DetectionError, DetectionEvidence, DetectionOutcome, ProbeContext,
};
use crate::probe_detector::contract::detector::{
    DetectorConfigError, ProbeDetector, ProbeDetectorConfig,
};
use crate::probe_detector::contract::identity::{DetectorId, DetectorPath};
use crate::probe_detector::contract::selection::DetectionSelector;

use super::Aarch64RustlsProbeDetectorConfig;
use super::common_state_pair_48_56::CommonStatePair4856ProbeDetector;
use super::common_state_pair_52_64::CommonStatePair5264ProbeDetector;
use crate::probe_detector::detector::tls::rustls::static_pattern::PatternPairProbeDetector;

pub(crate) struct Aarch64RustlsProbeDetector {
    path: DetectorPath,
    match_limit: usize,
    selector: DetectionSelector,
    candidates: [PatternPairProbeDetector; 2],
}

impl Aarch64RustlsProbeDetector {
    pub(crate) fn try_new(
        config: Aarch64RustlsProbeDetectorConfig,
    ) -> Result<Self, DetectorConfigError> {
        config.validate()?;
        Ok(Self {
            path: DetectorPath::root(DetectorId::new("tls"))
                .child(DetectorId::new("rustls"))
                .child(DetectorId::new("static-pattern"))
                .child(DetectorId::new("aarch64")),
            match_limit: config.match_limit,
            selector: DetectionSelector::new(config.selection),
            candidates: [
                CommonStatePair5264ProbeDetector::try_new(config.common_state_pair_52_64)?
                    .into_pattern_pair(),
                CommonStatePair4856ProbeDetector::try_new(config.common_state_pair_48_56)?
                    .into_pattern_pair(),
            ],
        })
    }

    pub(crate) fn register_executable_patterns(&self, image: &ElfImage) {
        if image.arch().as_str() != "aarch64" {
            return;
        }
        for candidate in &self.candidates {
            candidate.register_executable_patterns(image);
        }
    }
}

impl ProbeDetector for Aarch64RustlsProbeDetector {
    fn path(&self) -> &DetectorPath {
        &self.path
    }

    fn detect(&self, context: &ProbeContext<'_>) -> Result<DetectionOutcome, DetectionError> {
        if context.probe.image.arch().as_str() != "aarch64" {
            return Ok(DetectionOutcome::Inapplicable(
                DetectionEvidence::new(self.path.clone(), context.target.architecture.clone())
                    .rejected("aarch64 Rustls detector received another architecture"),
            ));
        }
        let mut outcomes = Vec::with_capacity(self.candidates.len());
        for candidate in &self.candidates {
            outcomes.push(candidate.detect_outcome(context, self.match_limit));
        }
        Ok(self.selector.select(
            DetectionEvidence::new(self.path.clone(), context.target.architecture.clone()),
            outcomes,
        ))
    }
}
