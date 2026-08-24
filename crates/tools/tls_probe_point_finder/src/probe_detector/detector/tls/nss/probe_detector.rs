use std::path::{Path, PathBuf};

use crate::plan::{ProbeSource, TlsProvider};
use crate::probe_detector::contract::detection::{
    DetectionError, DetectionEvidence, DetectionOutcome, LibraryCandidate, ProbeContext,
};
use crate::probe_detector::contract::detector::{
    DetectorConfigError, ProbeDetector, ProbeDetectorConfig,
};
use crate::probe_detector::contract::identity::{DetectorId, DetectorPath};

use super::NssProbeDetectorConfig;
use super::shared_library::NssSharedLibraryProbeDetector;

pub(crate) const NAME: &str = "nss";
pub(crate) const RESOLVER: &str = "nss-nspr-symbols";
pub(crate) const NSPR_PR_WRITE: &str = "PR_Write";
pub(crate) const NSPR_PR_SEND: &str = "PR_Send";
pub(crate) const NSPR_PR_READ: &str = "PR_Read";
pub(crate) const NSPR_PR_RECV: &str = "PR_Recv";
pub(crate) const SYMBOLS: &[&str] = &[NSPR_PR_WRITE, NSPR_PR_SEND, NSPR_PR_READ, NSPR_PR_RECV];

pub(crate) struct NssProbeDetector {
    path: DetectorPath,
    shared_library: NssSharedLibraryProbeDetector,
}

impl NssProbeDetector {
    pub(crate) fn try_new(config: NssProbeDetectorConfig) -> Result<Self, DetectorConfigError> {
        config.validate()?;
        let id = DetectorId::new(NAME);
        Ok(Self {
            path: DetectorPath::root(DetectorId::new("tls")).child(id.clone()),
            shared_library: NssSharedLibraryProbeDetector::try_new(config.shared_library)?,
        })
    }

    pub(crate) fn candidates(&self, target: &Path, libraries: &[PathBuf]) -> Vec<PathBuf> {
        self.shared_library.candidates(target, libraries)
    }
}

impl ProbeDetector for NssProbeDetector {
    fn path(&self) -> &DetectorPath {
        &self.path
    }

    fn detect(&self, context: &ProbeContext<'_>) -> Result<DetectionOutcome, DetectionError> {
        if context.request.requested_source == Some(ProbeSource::Executable)
            || context
                .request
                .requested_provider
                .is_some_and(|provider| provider != TlsProvider::Nss)
        {
            return Ok(DetectionOutcome::Inapplicable(
                DetectionEvidence::new(self.path.clone(), context.target.architecture.clone())
                    .rejected("NSS/NSPR shared-library detector excluded by request"),
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
                .rejected("NSS/NSPR shared-library symbols were not found"),
        ))
    }
}
