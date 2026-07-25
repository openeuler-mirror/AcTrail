use std::collections::BTreeMap;

use crate::ToolResult;
use crate::elf::{Arch, ElfImage};
use crate::plan::{ProbeSource, TlsProvider};
use crate::probe_detector::contract::detection::{
    DetectionError, DetectionEvidence, DetectionOutcome, ProbeContext, SymbolEvidence,
};
use crate::probe_detector::contract::detector::{
    DetectorConfigError, ProbeDetector, ProbeDetectorConfig,
};
use crate::probe_detector::contract::identity::{DetectorId, DetectorPath};
use crate::probe_detector::detector::tls::candidate::TlsProbeCandidateFactory;
use crate::probe_detector::detector::tls::openssl::{PROBE_SYMBOLS, REQUIRED_SYMBOLS, RESOLVER};

use super::OpenSslExecutableProbeDetectorConfig;

pub(crate) struct OpenSslExecutableProbeDetector {
    path: DetectorPath,
    expected_arch: Arch,
}

impl OpenSslExecutableProbeDetector {
    pub(crate) fn try_new(
        config: OpenSslExecutableProbeDetectorConfig,
        expected_arch: Arch,
    ) -> Result<Self, DetectorConfigError> {
        config.validate()?;
        Ok(Self {
            path: DetectorPath::root(DetectorId::new("tls"))
                .child(DetectorId::new("openssl"))
                .child(DetectorId::new(expected_arch.as_str()))
                .child(DetectorId::new("executable")),
            expected_arch,
        })
    }

    pub(crate) fn resolve(&self, image: &ElfImage) -> ToolResult<Option<BTreeMap<String, u64>>> {
        let symbols = image.unique_defined_symbol_values(PROBE_SYMBOLS)?;
        Ok(REQUIRED_SYMBOLS
            .iter()
            .all(|symbol| symbols.contains_key(*symbol))
            .then_some(symbols))
    }
}

impl ProbeDetector for OpenSslExecutableProbeDetector {
    fn path(&self) -> &DetectorPath {
        &self.path
    }

    fn detect(&self, context: &ProbeContext<'_>) -> Result<DetectionOutcome, DetectionError> {
        if context.probe.image.arch() != self.expected_arch {
            return Ok(DetectionOutcome::Inapplicable(
                DetectionEvidence::new(self.path.clone(), context.target.architecture.clone())
                    .rejected(format!(
                        "{} OpenSSL executable detector received another architecture",
                        self.expected_arch.as_str()
                    )),
            ));
        }
        if context.probe.source != ProbeSource::Executable {
            return Ok(DetectionOutcome::Inapplicable(
                DetectionEvidence::new(self.path.clone(), context.target.architecture.clone())
                    .rejected("OpenSSL executable detector requires an executable context"),
            ));
        }
        let Some(symbols) = self
            .resolve(context.probe.image)
            .map_err(|error| DetectionError::new(self.path.clone(), error.to_string()))?
        else {
            return Ok(DetectionOutcome::NoMatch(
                DetectionEvidence::new(self.path.clone(), context.target.architecture.clone())
                    .rejected("OpenSSL executable plaintext symbols were not found"),
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
        let candidate = TlsProbeCandidateFactory::new(
            context,
            self.path.clone(),
            TlsProvider::OpenSsl,
            RESOLVER,
        )
        .from_symbols(&symbols, evidence)
        .map_err(|error| DetectionError::new(self.path.clone(), error.to_string()))?;
        Ok(DetectionOutcome::Matched(candidate))
    }
}
