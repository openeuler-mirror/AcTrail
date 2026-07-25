use crate::probe_detector::contract::detection::{
    DetectionError, DetectionEvidence, DetectionOutcome, EvidenceFact, ProbeContext,
};
use crate::probe_detector::contract::detector::{
    DetectorConfigError, ProbeDetector, ProbeDetectorConfig,
};
use crate::probe_detector::contract::identity::{DetectorId, DetectorPath};
use crate::probe_detector::contract::selection::DetectionSelector;

use super::TlsProbeDetectorConfig;
use super::boringssl::BoringSslProbeDetector;
use super::gnutls::GnuTlsProbeDetector;
use super::go_tls::GoTlsProbeDetector;
use super::nss::NssProbeDetector;
use super::openssl::OpenSslProbeDetector;
use super::rustls::RustlsProbeDetector;

pub(crate) struct TlsProbeDetector {
    path: DetectorPath,
    selector: DetectionSelector,
    rustls: RustlsProbeDetector,
    boringssl: BoringSslProbeDetector,
    openssl: OpenSslProbeDetector,
    go_tls: GoTlsProbeDetector,
    gnutls: GnuTlsProbeDetector,
    nss: NssProbeDetector,
}

impl TlsProbeDetector {
    pub(crate) fn try_new(config: TlsProbeDetectorConfig) -> Result<Self, DetectorConfigError> {
        config.validate()?;
        let id = DetectorId::new("tls");
        Ok(Self {
            path: DetectorPath::root(id.clone()),
            selector: DetectionSelector::new(config.selection),
            rustls: RustlsProbeDetector::try_new(config.rustls)?,
            boringssl: BoringSslProbeDetector::try_new(config.boringssl)?,
            openssl: OpenSslProbeDetector::try_new(config.openssl)?,
            go_tls: GoTlsProbeDetector::try_new(config.go_tls)?,
            gnutls: GnuTlsProbeDetector::try_new(config.gnutls)?,
            nss: NssProbeDetector::try_new(config.nss)?,
        })
    }
}

impl ProbeDetector for TlsProbeDetector {
    fn path(&self) -> &DetectorPath {
        &self.path
    }

    fn detect(&self, context: &ProbeContext<'_>) -> Result<DetectionOutcome, DetectionError> {
        let detectors: [&dyn ProbeDetector; 6] = [
            &self.rustls,
            &self.openssl,
            &self.boringssl,
            &self.go_tls,
            &self.gnutls,
            &self.nss,
        ];
        let mut outcomes = Vec::new();
        let mut evidence =
            DetectionEvidence::new(self.path.clone(), context.target.architecture.clone());
        for detector in detectors {
            evidence.facts.push(EvidenceFact {
                key: "provider_detector".to_string(),
                value: detector.path().display(),
            });
            let outcome = detector.detect(context)?;
            let stop = self.selector.should_stop(&outcome);
            outcomes.push(outcome);
            if stop {
                break;
            }
        }
        Ok(self.selector.select(evidence, outcomes))
    }
}
