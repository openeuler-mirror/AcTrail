use std::collections::BTreeMap;

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

use super::RustlsSymbolProbeDetectorConfig;
use crate::probe_detector::detector::tls::rustls::{
    RUNTIME_BUFFER_PLAINTEXT_SYMBOL, RUNTIME_SYMBOLS, RUNTIME_TAKE_RECEIVED_PLAINTEXT_SYMBOL,
};

const DEMANGLED_BUFFER_PLAINTEXT_PREFIXES: &[&str] = &[
    "rustls::common_state::CommonState::buffer_plaintext",
    "<rustls::common_state::CommonState>::buffer_plaintext",
];
const DEMANGLED_TAKE_RECEIVED_PLAINTEXT_PREFIXES: &[&str] = &[
    "rustls::common_state::CommonState::take_received_plaintext",
    "<rustls::common_state::CommonState>::take_received_plaintext",
];

pub(crate) struct RustlsSymbolProbeDetector {
    path: DetectorPath,
}

pub(crate) struct DemangledPlaintextSymbols {
    pub(crate) runtime_symbols: BTreeMap<String, u64>,
    pub(crate) targets: Vec<DemangledPlaintextTarget>,
}

pub(crate) struct DemangledPlaintextTarget {
    pub(crate) runtime_symbol: &'static str,
    pub(crate) symbol: String,
    pub(crate) address: u64,
}

impl RustlsSymbolProbeDetector {
    pub(crate) fn try_new(
        config: RustlsSymbolProbeDetectorConfig,
    ) -> Result<Self, DetectorConfigError> {
        config.validate()?;
        let id = DetectorId::new("symbol");
        Ok(Self {
            path: DetectorPath::root(DetectorId::new("tls"))
                .child(DetectorId::new("rustls"))
                .child(id.clone()),
        })
    }

    pub(crate) fn resolve(
        &self,
        image: &ElfImage,
    ) -> ToolResult<Option<DemangledPlaintextSymbols>> {
        let mut targets = BTreeMap::<&'static str, DemangledPlaintextTarget>::new();
        for symbol in image.defined_function_symbols()? {
            if !symbol.raw_name.contains("rustls") {
                continue;
            }
            let demangled = format!("{:#}", rustc_demangle::demangle(&symbol.raw_name));
            if let Some(target) = Self::parse_target(
                &demangled,
                symbol.value,
                DEMANGLED_BUFFER_PLAINTEXT_PREFIXES,
                RUNTIME_BUFFER_PLAINTEXT_SYMBOL,
            ) {
                targets.insert(RUNTIME_BUFFER_PLAINTEXT_SYMBOL, target);
            } else if let Some(target) = Self::parse_target(
                &demangled,
                symbol.value,
                DEMANGLED_TAKE_RECEIVED_PLAINTEXT_PREFIXES,
                RUNTIME_TAKE_RECEIVED_PLAINTEXT_SYMBOL,
            ) {
                targets.insert(RUNTIME_TAKE_RECEIVED_PLAINTEXT_SYMBOL, target);
            }
        }
        if !RUNTIME_SYMBOLS
            .iter()
            .all(|symbol| targets.contains_key(symbol))
        {
            return Ok(None);
        }
        let runtime_symbols = RUNTIME_SYMBOLS
            .iter()
            .map(|symbol| {
                (
                    (*symbol).to_string(),
                    targets.get(symbol).expect("target checked above").address,
                )
            })
            .collect();
        let targets = RUNTIME_SYMBOLS
            .iter()
            .map(|symbol| targets.remove(symbol).expect("target checked above"))
            .collect();
        Ok(Some(DemangledPlaintextSymbols {
            runtime_symbols,
            targets,
        }))
    }

    fn parse_target(
        symbol: &str,
        address: u64,
        prefixes: &[&str],
        runtime_symbol: &'static str,
    ) -> Option<DemangledPlaintextTarget> {
        let matched = prefixes
            .iter()
            .any(|prefix| match symbol.strip_prefix(prefix) {
                Some("") => true,
                Some(tail) => tail.starts_with("::h"),
                None => false,
            });
        matched.then(|| DemangledPlaintextTarget {
            runtime_symbol,
            symbol: symbol.to_string(),
            address,
        })
    }
}

impl ProbeDetector for RustlsSymbolProbeDetector {
    fn path(&self) -> &DetectorPath {
        &self.path
    }

    fn detect(&self, context: &ProbeContext<'_>) -> Result<DetectionOutcome, DetectionError> {
        if context.probe.source != ProbeSource::Executable
            || context
                .request
                .requested_provider
                .is_some_and(|provider| provider != TlsProvider::Rustls)
        {
            return Ok(DetectionOutcome::Inapplicable(
                DetectionEvidence::new(self.path.clone(), context.target.architecture.clone())
                    .rejected("Rustls symbol detector excluded by request"),
            ));
        }
        let Some(symbols) = self
            .resolve(context.probe.image)
            .map_err(|error| DetectionError::new(self.path.clone(), error.to_string()))?
        else {
            return Ok(DetectionOutcome::NoMatch(
                DetectionEvidence::new(self.path.clone(), context.target.architecture.clone())
                    .rejected("Rustls plaintext symbols were not found"),
            ));
        };
        let mut evidence =
            DetectionEvidence::new(self.path.clone(), context.target.architecture.clone());
        evidence.symbols = symbols
            .targets
            .iter()
            .map(|target| SymbolEvidence {
                symbol: target.symbol.clone(),
                runtime_symbol: target.runtime_symbol.to_string(),
                virtual_address: target.address,
            })
            .collect();
        let candidate = TlsProbeCandidateFactory::new(
            context,
            self.path.clone(),
            TlsProvider::Rustls,
            crate::probe_detector::detector::tls::rustls::RESOLVER,
        )
        .from_symbols(&symbols.runtime_symbols, evidence)
        .map_err(|error| DetectionError::new(self.path.clone(), error.to_string()))?;
        Ok(DetectionOutcome::Matched(candidate))
    }
}
