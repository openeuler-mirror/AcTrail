use std::path::{Path, PathBuf};

use crate::elf::ElfImage;
use crate::plan::{ProbeSource, TlsProvider};
use crate::probe_detector::contract::detection::{
    DetectionError, DetectionEvidence, DetectionOutcome, LibraryCandidate, ProbeContext,
};
use crate::probe_detector::contract::detector::{
    DetectorConfigError, ProbeDetector, ProbeDetectorConfig,
};
use crate::probe_detector::contract::identity::{DetectorId, DetectorPath};
use crate::probe_detector::contract::selection::{DetectionSelector, SelectionPolicy};

use super::BoringSslProbeDetectorConfig;
use super::aarch64::Aarch64BoringSslProbeDetector;
use super::x86_64::X86_64BoringSslProbeDetector;

pub(crate) const NAME: &str = "boringssl";
pub(crate) const MAP_SYMBOLS_X86_64: &[&str] = &["SSL_do_handshake", "SSL_read", "SSL_write"];
pub(crate) const MAP_SYMBOLS_AARCH64: &[&str] = &["SSL_read", "SSL_write"];
pub(crate) const SYMBOL_MAP_RESOLVER: &str = "bun-static-boringssl";
pub(crate) const SHARED_SYMBOL_MAP_RESOLVER: &str = "boringssl-shared-symbols";
pub(crate) const STATIC_RESOLVER: &str = "boringssl-static";

pub(crate) struct BoringSslProbeDetector {
    path: DetectorPath,
    x86_64: X86_64BoringSslProbeDetector,
    aarch64: Aarch64BoringSslProbeDetector,
}

impl BoringSslProbeDetector {
    pub(crate) fn try_new(
        config: BoringSslProbeDetectorConfig,
    ) -> Result<Self, DetectorConfigError> {
        config.validate()?;
        let id = DetectorId::new(NAME);
        Ok(Self {
            path: DetectorPath::root(DetectorId::new("tls")).child(id),
            x86_64: X86_64BoringSslProbeDetector::try_new(config.x86_64)?,
            aarch64: Aarch64BoringSslProbeDetector::try_new(config.aarch64)?,
        })
    }

    fn arch_outcome(&self, context: &ProbeContext<'_>) -> Result<DetectionOutcome, DetectionError> {
        let outcomes = vec![self.x86_64.detect(context)?, self.aarch64.detect(context)?];
        Ok(
            DetectionSelector::new(SelectionPolicy::SelectApplicable).select(
                DetectionEvidence::new(self.path.clone(), context.target.architecture.clone()),
                outcomes,
            ),
        )
    }

    fn library_candidates(&self, context: &ProbeContext<'_>) -> Vec<PathBuf> {
        let explicit = context.request.requested_provider == Some(TlsProvider::BoringSsl);
        let mut candidates = context
            .request
            .libraries
            .iter()
            .filter(|path| explicit || !Self::is_libssl(path))
            .cloned()
            .collect::<Vec<_>>();
        if Self::is_shared_object(context.target_image.path())
            && (explicit || !Self::is_libssl(context.target_image.path()))
            && !candidates
                .iter()
                .any(|path| path == context.target_image.path())
        {
            candidates.push(context.target_image.path().to_path_buf());
        }
        candidates
    }

    fn is_shared_object(path: &Path) -> bool {
        path.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.contains(".so"))
    }

    fn is_libssl(path: &Path) -> bool {
        path.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("libssl") && name.contains(".so"))
    }
}

impl ProbeDetector for BoringSslProbeDetector {
    fn path(&self) -> &DetectorPath {
        &self.path
    }

    fn detect(&self, context: &ProbeContext<'_>) -> Result<DetectionOutcome, DetectionError> {
        if context
            .request
            .requested_provider
            .is_some_and(|provider| provider != TlsProvider::BoringSsl)
        {
            return Ok(DetectionOutcome::Inapplicable(
                DetectionEvidence::new(self.path.clone(), context.target.architecture.clone())
                    .rejected("BoringSSL detector excluded by request"),
            ));
        }
        if context.request.requested_source != Some(ProbeSource::SharedLibrary) {
            let outcome = self.arch_outcome(context)?;
            if matches!(outcome, DetectionOutcome::Matched(_)) {
                return Ok(outcome);
            }
        }
        if context.request.requested_source != Some(ProbeSource::Executable) {
            for candidate in self.library_candidates(context) {
                let image = ElfImage::parse(&candidate)
                    .map_err(|error| DetectionError::new(self.path.clone(), error.to_string()))?;
                if image.arch() != context.target_image.arch() {
                    continue;
                }
                let library = LibraryCandidate {
                    path: candidate,
                    note: None,
                };
                let library_context = context.for_library(&image, &library);
                let outcome = self.arch_outcome(&library_context)?;
                if matches!(outcome, DetectionOutcome::Matched(_)) {
                    return Ok(outcome);
                }
            }
        }
        Ok(DetectionOutcome::NoMatch(
            DetectionEvidence::new(self.path.clone(), context.target.architecture.clone())
                .rejected("BoringSSL symbols and static patterns were not found"),
        ))
    }
}
