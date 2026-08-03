use std::collections::BTreeMap;

use crate::ToolResult;
use crate::elf::{Arch, ElfImage};
use crate::pattern_search::ExactPatternSearch;
use crate::plan::{ProbeSource, TlsProvider};
use crate::probe_detector::contract::detection::{
    DetectionError, DetectionEvidence, DetectionOutcome, EvidenceFact, ProbeContext, SymbolEvidence,
};
use crate::probe_detector::contract::detector::{
    DetectorConfigError, ProbeDetector, ProbeDetectorConfig,
};
use crate::probe_detector::contract::identity::{DetectorId, DetectorPath};
use crate::probe_detector::detector::tls::candidate::TlsProbeCandidateFactory;
use crate::probe_detector::detector::tls::openssl::{PROBE_SYMBOLS, REQUIRED_SYMBOLS, RESOLVER};
use crate::{BinaryIdentity, BinaryIdentityTypeCode, ToolError};

use super::OpenSslExecutableProbeDetectorConfig;

const CODEX_0146_IDENTITY: &str =
    "6948d0811ec18dab404ee6949296b85dc192126a6033ab17918b9b61d8bdc168";
const CODEX_SSL_READ_EX_PATTERN: &[u8] = &[
    0x55, 0x48, 0x89, 0xe5, 0xe8, 0xc7, 0xfb, 0xff, 0xff, 0x31, 0xc9, 0x85, 0xc0, 0x0f, 0x4e, 0xc1,
    0x5d, 0xc3,
];
const CODEX_SSL_WRITE_EX_PATTERN: &[u8] = &[
    0x55, 0x48, 0x89, 0xe5, 0x53, 0x50, 0x49, 0x89, 0xc8, 0x31, 0xdb, 0x31, 0xc9, 0xe8, 0xbe, 0xfd,
    0xff, 0xff, 0x85, 0xc0, 0x0f, 0x4e, 0xc3, 0x48, 0x83, 0xc4, 0x08, 0x5b, 0x5d, 0xc3,
];

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
        if REQUIRED_SYMBOLS
            .iter()
            .all(|symbol| symbols.contains_key(*symbol))
        {
            return Ok(Some(symbols));
        }
        self.resolve_verified_static(image)
    }

    fn resolve_verified_static(
        &self,
        image: &ElfImage,
    ) -> ToolResult<Option<BTreeMap<String, u64>>> {
        if self.expected_arch != Arch::X86_64
            || image.identity()
                != &BinaryIdentity::try_new(
                    BinaryIdentityTypeCode::ElfExecutableSampleSha256V1,
                    CODEX_0146_IDENTITY,
                )
                .map_err(|error| ToolError::new(error.to_string()))?
        {
            return Ok(None);
        }
        let patterns = [
            (
                crate::probe_detector::detector::tls::openssl::SSL_READ_EX,
                CODEX_SSL_READ_EX_PATTERN,
            ),
            (
                crate::probe_detector::detector::tls::openssl::SSL_WRITE_EX,
                CODEX_SSL_WRITE_EX_PATTERN,
            ),
        ];
        let executable_ranges = image.executable_file_ranges()?;
        let mut symbols = BTreeMap::new();
        for (symbol, pattern) in patterns {
            let matches = ExactPatternSearch::new(pattern).map_or_else(Vec::new, |search| {
                search.find_all_in_file_ranges(&executable_ranges)
            });
            if matches.len() != 1 {
                return Err(ToolError::new(format!(
                    "verified Codex OpenSSL {symbol} pattern match count={}",
                    matches.len()
                )));
            }
            symbols.insert(
                symbol.to_string(),
                image.virtual_address_for_file_offset(matches[0] as u64)?,
            );
        }
        Ok(Some(symbols))
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
        if context.probe.image.identity().identity == CODEX_0146_IDENTITY {
            evidence.facts.extend([
                EvidenceFact {
                    key: "resolver_mode".to_string(),
                    value: "verified-static-pattern".to_string(),
                },
                EvidenceFact {
                    key: "verified_runtime".to_string(),
                    value: "Codex 0.146.0 embedded OpenSSL 3.6.3".to_string(),
                },
                EvidenceFact {
                    key: "verified_identity_status".to_string(),
                    value: "exact-match".to_string(),
                },
                EvidenceFact {
                    key: "verified_evidence_source".to_string(),
                    value: "native Codex real-agent uprobe hit profile".to_string(),
                },
            ]);
        }
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
