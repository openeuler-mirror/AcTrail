use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::ToolResult;
use crate::elf::ElfImage;
use crate::plan::{ProbeSource, TlsProvider};
use crate::probe_detector::contract::detection::{
    DetectionError, DetectionEvidence, DetectionOutcome, ProbeContext, SymbolEvidence,
};
use crate::probe_detector::contract::detector::{
    DetectorConfigError, ProbeDetector, ProbeDetectorConfig,
};
use crate::probe_detector::contract::identity::{DetectorId, DetectorPath};
use crate::probe_detector::detector::tls::candidate::TlsProbeCandidateFactory;
use crate::probe_detector::detector::tls::nss::{RESOLVER, SYMBOLS};

use super::NssSharedLibraryProbeDetectorConfig;

pub(crate) struct NssSharedLibraryProbeDetector {
    path: DetectorPath,
}

impl NssSharedLibraryProbeDetector {
    pub(crate) fn try_new(
        config: NssSharedLibraryProbeDetectorConfig,
    ) -> Result<Self, DetectorConfigError> {
        config.validate()?;
        Ok(Self {
            path: DetectorPath::root(DetectorId::new("tls"))
                .child(DetectorId::new("nss"))
                .child(DetectorId::new("shared-library")),
        })
    }

    pub(crate) fn candidates(&self, target: &Path, libraries: &[PathBuf]) -> Vec<PathBuf> {
        Self::unique_candidates(target, libraries, "libnspr4")
    }

    pub(crate) fn resolve_symbols(
        &self,
        image: &ElfImage,
    ) -> ToolResult<Option<BTreeMap<String, u64>>> {
        let symbols = image.unique_defined_symbol_values(SYMBOLS)?;
        Ok(SYMBOLS
            .iter()
            .all(|symbol| symbols.contains_key(*symbol))
            .then_some(symbols))
    }

    fn unique_candidates(target: &Path, libraries: &[PathBuf], prefix: &str) -> Vec<PathBuf> {
        let mut candidates = Vec::new();
        for path in libraries {
            if !candidates.contains(path) {
                candidates.push(path.clone());
            }
        }
        if target
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(prefix) && name.contains(".so"))
            && !candidates.iter().any(|path| path == target)
        {
            candidates.push(target.to_path_buf());
        }
        candidates
    }
}

impl ProbeDetector for NssSharedLibraryProbeDetector {
    fn path(&self) -> &DetectorPath {
        &self.path
    }

    fn detect(&self, context: &ProbeContext<'_>) -> Result<DetectionOutcome, DetectionError> {
        if context.probe.source != ProbeSource::SharedLibrary {
            return Ok(DetectionOutcome::Inapplicable(
                DetectionEvidence::new(self.path.clone(), context.target.architecture.clone())
                    .rejected("NSS/NSPR resolver requires a shared-library context"),
            ));
        }
        let Some(symbols) = self
            .resolve_symbols(context.probe.image)
            .map_err(|error| DetectionError::new(self.path.clone(), error.to_string()))?
        else {
            return Ok(DetectionOutcome::NoMatch(
                DetectionEvidence::new(self.path.clone(), context.target.architecture.clone())
                    .rejected("NSS/NSPR plaintext symbols were not found"),
            ));
        };
        let mut evidence =
            DetectionEvidence::new(self.path.clone(), context.probe.image.arch().as_str());
        evidence.symbols = symbols
            .iter()
            .map(|(symbol, virtual_address)| SymbolEvidence {
                symbol: symbol.clone(),
                runtime_symbol: symbol.clone(),
                virtual_address: *virtual_address,
            })
            .collect();
        let candidate =
            TlsProbeCandidateFactory::new(context, self.path.clone(), TlsProvider::Nss, RESOLVER)
                .from_symbols(&symbols, evidence)
                .map_err(|error| DetectionError::new(self.path.clone(), error.to_string()))?;
        Ok(DetectionOutcome::Matched(candidate))
    }
}
