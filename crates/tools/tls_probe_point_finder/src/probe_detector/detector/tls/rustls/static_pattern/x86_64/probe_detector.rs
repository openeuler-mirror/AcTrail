use crate::elf::ElfImage;
use crate::probe_detector::contract::detection::{
    DetectionError, DetectionEvidence, DetectionOutcome, ProbeContext,
};
use crate::probe_detector::contract::detector::{
    DetectorConfigError, ProbeDetector, ProbeDetectorConfig,
};
use crate::probe_detector::contract::identity::{DetectorId, DetectorPath};
use crate::probe_detector::contract::selection::DetectionSelector;

use super::X86_64RustlsProbeDetectorConfig;
use super::common_state_pair_27_31::CommonStatePair2731ProbeDetector;
use super::common_state_pair_27_32::CommonStatePair2732ProbeDetector;
use super::common_state_pair_41_32::CommonStatePair4132ProbeDetector;
use crate::probe_detector::detector::tls::rustls::static_pattern::PatternPairProbeDetector;

pub(crate) struct X86_64RustlsProbeDetector {
    path: DetectorPath,
    match_limit: usize,
    selector: DetectionSelector,
    candidates: [PatternPairProbeDetector; 3],
}

impl X86_64RustlsProbeDetector {
    pub(crate) fn try_new(
        config: X86_64RustlsProbeDetectorConfig,
    ) -> Result<Self, DetectorConfigError> {
        config.validate()?;
        Ok(Self {
            path: DetectorPath::root(DetectorId::new("tls"))
                .child(DetectorId::new("rustls"))
                .child(DetectorId::new("static-pattern"))
                .child(DetectorId::new("x86_64")),
            match_limit: config.match_limit,
            selector: DetectionSelector::new(config.selection),
            candidates: [
                CommonStatePair2731ProbeDetector::try_new(config.common_state_pair_27_31)?
                    .into_pattern_pair(),
                CommonStatePair2732ProbeDetector::try_new(config.common_state_pair_27_32)?
                    .into_pattern_pair(),
                CommonStatePair4132ProbeDetector::try_new(config.common_state_pair_41_32)?
                    .into_pattern_pair(),
            ],
        })
    }

    pub(crate) fn register_executable_patterns(&self, image: &ElfImage) {
        if image.arch().as_str() != "x86_64" {
            return;
        }
        for candidate in &self.candidates {
            candidate.register_executable_patterns(image);
        }
    }
}

impl ProbeDetector for X86_64RustlsProbeDetector {
    fn path(&self) -> &DetectorPath {
        &self.path
    }

    fn detect(&self, context: &ProbeContext<'_>) -> Result<DetectionOutcome, DetectionError> {
        if context.probe.image.arch().as_str() != "x86_64" {
            return Ok(DetectionOutcome::Inapplicable(
                DetectionEvidence::new(self.path.clone(), context.target.architecture.clone())
                    .rejected("x86_64 Rustls detector received another architecture"),
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
