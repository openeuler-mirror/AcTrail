use std::path::{Path, PathBuf};

use crate::plan::{ProbeSource, TlsProvider};
use crate::probe_detector::contract::detection::{
    DetectionError, DetectionEvidence, DetectionOutcome, LibraryCandidate, ProbeContext,
};
use crate::probe_detector::contract::detector::{
    DetectorConfigError, ProbeDetector, ProbeDetectorConfig,
};
use crate::probe_detector::contract::identity::{DetectorId, DetectorPath};

use super::GnuTlsProbeDetectorConfig;
use super::shared_library::GnuTlsSharedLibraryProbeDetector;

pub(crate) const NAME: &str = "gnutls";
pub(crate) const RESOLVER: &str = "gnutls-symbols";
pub(crate) const RECORD_SEND: &str = "gnutls_record_send";
pub(crate) const RECORD_RECV: &str = "gnutls_record_recv";
pub(crate) const SYMBOLS: &[&str] = &[RECORD_SEND, RECORD_RECV];

pub(crate) struct GnuTlsProbeDetector {
    path: DetectorPath,
    shared_library: GnuTlsSharedLibraryProbeDetector,
}

impl GnuTlsProbeDetector {
    pub(crate) fn try_new(config: GnuTlsProbeDetectorConfig) -> Result<Self, DetectorConfigError> {
        config.validate()?;
        let id = DetectorId::new(NAME);
        Ok(Self {
            path: DetectorPath::root(DetectorId::new("tls")).child(id.clone()),
            shared_library: GnuTlsSharedLibraryProbeDetector::try_new(config.shared_library)?,
        })
    }

    pub(crate) fn candidates(&self, target: &Path, libraries: &[PathBuf]) -> Vec<PathBuf> {
        self.shared_library.candidates(target, libraries)
    }
}

impl ProbeDetector for GnuTlsProbeDetector {
    fn path(&self) -> &DetectorPath {
        &self.path
    }

    fn detect(&self, context: &ProbeContext<'_>) -> Result<DetectionOutcome, DetectionError> {
        if context.request.requested_source == Some(ProbeSource::Executable)
            || context
                .request
                .requested_provider
                .is_some_and(|provider| provider != TlsProvider::GnuTls)
        {
            return Ok(DetectionOutcome::Inapplicable(
                DetectionEvidence::new(self.path.clone(), context.target.architecture.clone())
                    .rejected("GnuTLS shared-library detector excluded by request"),
            ));
        }
        for candidate in self.candidates(context.target_image.path(), &context.request.libraries) {
            let image = context
                .parse_probe_image(&candidate)
                .map_err(|error| DetectionError::new(self.path.clone(), error.to_string()))?;
            if image.arch() != context.target_image.arch() {
                continue;
            }
            let library = LibraryCandidate {
                path: candidate,
                note: None,
            };
            let library_context = context.for_library(&image, &library);
            let outcome = self.shared_library.detect(&library_context)?;
            if matches!(outcome, DetectionOutcome::Matched(_)) {
                return Ok(outcome);
            }
        }
        Ok(DetectionOutcome::NoMatch(
            DetectionEvidence::new(self.path.clone(), context.target.architecture.clone())
                .rejected("GnuTLS shared-library symbols were not found"),
        ))
    }
}
