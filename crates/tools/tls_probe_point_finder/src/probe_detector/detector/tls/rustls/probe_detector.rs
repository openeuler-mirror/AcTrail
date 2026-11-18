use crate::plan::{ProbeSource, TlsProvider};
use crate::probe_detector::contract::candidate::ProbeCandidate;
use crate::probe_detector::contract::detection::{
    DetectionError, DetectionEvidence, DetectionOutcome, ProbeConsumer, ProbeContext,
};
use crate::probe_detector::contract::detector::{
    DetectorConfigError, ProbeDetector, ProbeDetectorConfig,
};
use crate::probe_detector::contract::identity::{DetectorId, DetectorPath};
use crate::probe_detector::contract::selection::DetectionSelector;

use super::RustlsProbeDetectorConfig;
use super::handshake_locator::RustlsHandshakeLocator;
use super::static_pattern::RustlsStaticPatternProbeDetector;
use super::symbol::RustlsSymbolProbeDetector;
use crate::probe_detector::contract::capability::ConsumerCapability;
use crate::probe_detector::detector::tls::ExecutablePatternRegistration;

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
    handshake: RustlsHandshakeLocator,
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
            handshake: RustlsHandshakeLocator::try_new()?,
        })
    }

    fn requires_native(context: &ProbeContext<'_>) -> bool {
        context.probe.image.arch().as_str() == "aarch64"
            && context.probe.source == ProbeSource::Executable
            && context.request.requested_source != Some(ProbeSource::SharedLibrary)
            && context
                .request
                .requested_provider
                .is_none_or(|provider| provider == TlsProvider::Rustls)
            && (matches!(
                context.request.consumer,
                ProbeConsumer::Direct | ProbeConsumer::PlanOnly
            ) || context.request.consumer == ProbeConsumer::Daemon
                && !context.target_image.has_interpreter())
    }

    fn enhance(&self, context: &ProbeContext<'_>, outcome: DetectionOutcome) -> DetectionOutcome {
        let mut candidate = match outcome {
            DetectionOutcome::Matched(mut candidate) => {
                if !candidate.evidence.children.is_empty() {
                    self.enhance_children(context, &mut candidate.evidence);
                    let children = std::mem::take(&mut candidate.evidence.children);
                    let result = Self::reuse_candidate(&mut candidate, &children);
                    candidate.evidence.children = children;
                    return match result {
                        Ok(()) => DetectionOutcome::Matched(candidate),
                        Err(reason) => {
                            DetectionOutcome::NoMatch(candidate.evidence.rejected(reason))
                        }
                    };
                }
                candidate
            }
            DetectionOutcome::Collected(mut evidence) => {
                self.enhance_children(context, &mut evidence);
                return DetectionOutcome::Collected(evidence);
            }
            DetectionOutcome::Ambiguous(mut group) => {
                self.enhance_children(context, &mut group.evidence);
                let mut accepted = Vec::new();
                for mut candidate in group.candidates {
                    if Self::reuse_candidate(&mut candidate, &group.evidence.children).is_ok() {
                        candidate.evidence.children.clear();
                        accepted.push(candidate);
                    }
                }
                if accepted.len() == 1 {
                    let mut candidate = accepted.remove(0);
                    candidate.evidence.children = group.evidence.children;
                    return DetectionOutcome::Matched(candidate);
                }
                if accepted.is_empty() {
                    return DetectionOutcome::NoMatch(
                        group
                            .evidence
                            .rejected("no verified rustls handshake candidate"),
                    );
                }
                group.candidates = accepted;
                return DetectionOutcome::Ambiguous(group);
            }
            other => return other,
        };
        let _ = super::RustlsConnectionLayout::augment(
            context.probe.image,
            &self.handshake,
            &mut candidate.points,
            &mut candidate.evidence,
        );
        candidate.capability.consumer = ConsumerCapability::evaluate(
            candidate.capability.consumer.key.clone(),
            &candidate.points,
            candidate.capability.complete_plaintext_closure,
            context.target_image.has_interpreter(),
        );
        DetectionOutcome::Matched(candidate)
    }

    fn enhance_children(&self, context: &ProbeContext<'_>, evidence: &mut DetectionEvidence) {
        evidence.children = std::mem::take(&mut evidence.children)
            .into_iter()
            .map(|child| self.enhance(context, child))
            .collect();
    }

    fn reuse_candidate(
        candidate: &mut ProbeCandidate,
        children: &[DetectionOutcome],
    ) -> Result<(), String> {
        let selected = Self::selected_candidate(candidate, children)
            .ok_or_else(|| "selected rustls candidate has no enhanced evidence".to_string())?;
        match selected {
            Ok(enhanced) => {
                candidate.points = enhanced.points.clone();
                candidate.capability = enhanced.capability.clone();
                candidate.evidence.facts = enhanced.evidence.facts.clone();
                Ok(())
            }
            Err(evidence) => Err(evidence
                .rejection
                .clone()
                .unwrap_or_else(|| "selected rustls handshake candidate was rejected".to_string())),
        }
    }

    fn selected_candidate<'a>(
        selected: &ProbeCandidate,
        children: &'a [DetectionOutcome],
    ) -> Option<Result<&'a ProbeCandidate, &'a DetectionEvidence>> {
        for child in children {
            let evidence = match child {
                DetectionOutcome::Matched(candidate) => {
                    if candidate.detector_path == selected.detector_path
                        && candidate.binary == selected.binary
                        && candidate.source == selected.source
                    {
                        return Some(Ok(candidate));
                    }
                    &candidate.evidence
                }
                DetectionOutcome::NoMatch(evidence) => {
                    if evidence.detector_path == selected.detector_path {
                        return Some(Err(evidence));
                    }
                    evidence
                }
                DetectionOutcome::Collected(evidence)
                | DetectionOutcome::Inapplicable(evidence) => evidence,
                DetectionOutcome::Ambiguous(group) => &group.evidence,
            };
            if let Some(candidate) = Self::selected_candidate(selected, &evidence.children) {
                return Some(candidate);
            }
        }
        None
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
        let native = Self::requires_native(context);
        for detector in detectors {
            let outcome = detector.detect(context)?;
            let outcome = if native {
                self.enhance(context, outcome)
            } else {
                outcome
            };
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

impl ExecutablePatternRegistration for RustlsProbeDetector {
    fn register_executable_patterns(&self, context: &ProbeContext<'_>) {
        super::RustlsConnectionLayout::register(context.probe.image);
        if Self::requires_native(context) {
            self.handshake.register(context.probe.image);
        }
        self.static_pattern.register_executable_patterns(context);
    }

    fn detector_path(&self) -> &DetectorPath {
        &self.path
    }
}
