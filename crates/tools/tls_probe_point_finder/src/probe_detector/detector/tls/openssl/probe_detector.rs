use std::path::PathBuf;

use crate::ToolResult;
use crate::elf::{Arch, ElfImage};
use crate::plan::{ProbeSource, TlsProvider};
use crate::probe_detector::contract::detection::{
    DetectionError, DetectionEvidence, DetectionOutcome, LibraryCandidate, ProbeContext,
};
use crate::probe_detector::contract::detector::{
    DetectorConfigError, ProbeDetector, ProbeDetectorConfig,
};
use crate::probe_detector::contract::identity::{DetectorId, DetectorPath};
use crate::probe_detector::contract::selection::{DetectionSelector, SelectionPolicy};

use super::OpenSslProbeDetectorConfig;
use super::common::executable::OpenSslExecutableProbeDetector;
use super::common::shared_library::OpenSslSharedLibraryProbeDetector;
use super::common::shared_library::discovery::LibrarySearch;
use super::{aarch64, x86_64};
use crate::probe_detector::detector::tls::ExecutablePatternRegistration;

pub(crate) const NAME: &str = "openssl";
pub(crate) const RESOLVER: &str = "openssl-symbols";
pub(crate) const SSL_READ: &str = "SSL_read";
pub(crate) const SSL_WRITE: &str = "SSL_write";
pub(crate) const SSL_READ_EX: &str = "SSL_read_ex";
pub(crate) const SSL_WRITE_EX: &str = "SSL_write_ex";
pub(crate) const SSL_WRITE_EX2: &str = "SSL_write_ex2";
pub(crate) const REQUIRED_SYMBOLS: &[&str] = &[SSL_READ, SSL_WRITE, SSL_READ_EX, SSL_WRITE_EX];
pub(crate) const PROBE_SYMBOLS: &[&str] = &[
    SSL_READ,
    SSL_WRITE,
    SSL_READ_EX,
    SSL_WRITE_EX,
    SSL_WRITE_EX2,
];

pub(crate) struct OpenSslProbeDetector {
    path: DetectorPath,
    x86_64: OpenSslArchitectureBranch,
    aarch64: OpenSslArchitectureBranch,
}

struct OpenSslArchitectureBranch {
    path: DetectorPath,
    expected_arch: Arch,
    executable: OpenSslExecutableProbeDetector,
    shared_library: OpenSslSharedLibraryProbeDetector,
}

impl OpenSslProbeDetector {
    pub(crate) fn try_new(config: OpenSslProbeDetectorConfig) -> Result<Self, DetectorConfigError> {
        config.validate()?;
        let id = DetectorId::new(NAME);
        Ok(Self {
            path: DetectorPath::root(DetectorId::new("tls")).child(id.clone()),
            x86_64: OpenSslArchitectureBranch::new(
                Arch::X86_64,
                x86_64::executable::OpenSslExecutableProbeDetector::try_new(
                    config.executable.clone(),
                    Arch::X86_64,
                )?,
                x86_64::shared_library::OpenSslSharedLibraryProbeDetector::try_new(
                    config.shared_library.clone(),
                    Arch::X86_64,
                )?,
            ),
            aarch64: OpenSslArchitectureBranch::new(
                Arch::Aarch64,
                aarch64::executable::OpenSslExecutableProbeDetector::try_new(
                    config.executable,
                    Arch::Aarch64,
                )?,
                aarch64::shared_library::OpenSslSharedLibraryProbeDetector::try_new(
                    config.shared_library,
                    Arch::Aarch64,
                )?,
            ),
        })
    }
}

impl OpenSslArchitectureBranch {
    fn new(
        arch: Arch,
        executable: OpenSslExecutableProbeDetector,
        shared_library: OpenSslSharedLibraryProbeDetector,
    ) -> Self {
        Self {
            path: DetectorPath::root(DetectorId::new("tls"))
                .child(DetectorId::new(NAME))
                .child(DetectorId::new(arch.as_str())),
            expected_arch: arch,
            executable,
            shared_library,
        }
    }

    pub(crate) fn discover_libraries(
        &self,
        image: &ElfImage,
        libraries: &[PathBuf],
        library_search_dirs: &[PathBuf],
        include_transitive: bool,
    ) -> ToolResult<LibrarySearch> {
        self.shared_library
            .discover(image, libraries, library_search_dirs, include_transitive)
    }

    fn shared_library_outcome(
        &self,
        context: &ProbeContext<'_>,
    ) -> Result<DetectionOutcome, DetectionError> {
        let search = self
            .discover_libraries(
                context.target_image,
                &context.request.libraries,
                &context.request.library_search_dirs,
                true,
            )
            .map_err(|error| DetectionError::new(self.path.clone(), error.to_string()))?;
        let discovery_notices = search.notices.join("; ");
        let mut outcomes = Vec::new();
        for discovered in search.candidates {
            let image = ElfImage::parse(&discovered.path)
                .map_err(|error| DetectionError::new(self.path.clone(), error.to_string()))?;
            if image.arch() != context.target_image.arch() {
                continue;
            }
            let mut notes = vec![format!("confidence={}", discovered.confidence)];
            if let Some(note) = discovered.note {
                notes.push(note);
            }
            if !discovery_notices.is_empty() {
                notes.push(format!("discovery_notices={discovery_notices}"));
            }
            let library = LibraryCandidate {
                path: discovered.path,
                note: Some(notes.join(" ")),
            };
            let library_context = context.for_library(&image, &library);
            let outcome = self.shared_library.detect(&library_context)?;
            outcomes.push(outcome);
        }
        Ok(DetectionSelector::new(SelectionPolicy::UniqueMatch).select(
            DetectionEvidence::new(
                self.path.child(DetectorId::new("shared-library")),
                context.target.architecture.clone(),
            ),
            outcomes,
        ))
    }
}

impl ProbeDetector for OpenSslArchitectureBranch {
    fn path(&self) -> &DetectorPath {
        &self.path
    }

    fn detect(&self, context: &ProbeContext<'_>) -> Result<DetectionOutcome, DetectionError> {
        if context.target_image.arch() != self.expected_arch {
            return Ok(DetectionOutcome::Inapplicable(
                DetectionEvidence::new(self.path.clone(), context.target.architecture.clone())
                    .rejected(format!(
                        "{} OpenSSL branch received another architecture",
                        self.expected_arch.as_str()
                    )),
            ));
        }
        let selector = DetectionSelector::new(SelectionPolicy::FirstComplete);
        let mut outcomes = Vec::new();
        if context.request.requested_source != Some(ProbeSource::SharedLibrary) {
            let outcome = self.executable.detect(context)?;
            let stop = selector.should_stop(&outcome);
            outcomes.push(outcome);
            if stop {
                return Ok(selector.select(
                    DetectionEvidence::new(self.path.clone(), context.target.architecture.clone()),
                    outcomes,
                ));
            }
        }
        if context.request.requested_source != Some(ProbeSource::Executable) {
            outcomes.push(self.shared_library_outcome(context)?);
        }
        Ok(selector.select(
            DetectionEvidence::new(self.path.clone(), context.target.architecture.clone()),
            outcomes,
        ))
    }
}

impl ProbeDetector for OpenSslProbeDetector {
    fn path(&self) -> &DetectorPath {
        &self.path
    }

    fn detect(&self, context: &ProbeContext<'_>) -> Result<DetectionOutcome, DetectionError> {
        if context
            .request
            .requested_provider
            .is_some_and(|provider| provider != TlsProvider::OpenSsl)
        {
            return Ok(DetectionOutcome::Inapplicable(
                DetectionEvidence::new(self.path.clone(), context.target.architecture.clone())
                    .rejected("OpenSSL detector excluded by request"),
            ));
        }
        let outcomes = vec![self.x86_64.detect(context)?, self.aarch64.detect(context)?];
        Ok(
            DetectionSelector::new(SelectionPolicy::SelectApplicable).select(
                DetectionEvidence::new(self.path.clone(), context.target.architecture.clone()),
                outcomes,
            ),
        )
    }
}

impl ExecutablePatternRegistration for OpenSslProbeDetector {
    fn register_executable_patterns(&self, context: &ProbeContext<'_>) {
        if context
            .request
            .requested_provider
            .is_some_and(|provider| provider != TlsProvider::OpenSsl)
        {
            return;
        }
        self.x86_64.executable.register_executable_patterns(context);
        self.aarch64
            .executable
            .register_executable_patterns(context);
    }

    fn detector_path(&self) -> &DetectorPath {
        &self.path
    }
}
