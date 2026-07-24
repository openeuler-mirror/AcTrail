use std::collections::BTreeMap;

use crate::plan::TlsProvider;
use crate::probe_detector::contract::detection::{
    DetectionError, DetectionEvidence, DetectionOutcome, EvidenceLocation, PatternEvidence,
    ProbeContext, SymbolEvidence,
};
use crate::probe_detector::contract::identity::DetectorPath;
use crate::probe_detector::detector::tls::boringssl::common::StaticPatternDetection;
use crate::probe_detector::detector::tls::candidate::TlsProbeCandidateFactory;

pub(in crate::probe_detector::detector::tls::boringssl) struct BoringSslOutcomeFactory;

impl BoringSslOutcomeFactory {
    pub(in crate::probe_detector::detector::tls::boringssl) fn symbols(
        context: &ProbeContext<'_>,
        path: DetectorPath,
        symbols: BTreeMap<String, u64>,
        resolver: &'static str,
    ) -> Result<DetectionOutcome, DetectionError> {
        let symbols = symbols
            .into_iter()
            .filter(|(symbol, _)| Self::is_payload_symbol(symbol))
            .collect::<BTreeMap<_, _>>();
        let mut evidence =
            DetectionEvidence::new(path.clone(), context.probe.image.arch().as_str());
        evidence.symbols = symbols
            .iter()
            .map(|(symbol, virtual_address)| SymbolEvidence {
                symbol: symbol.clone(),
                runtime_symbol: symbol.clone(),
                virtual_address: *virtual_address,
            })
            .collect();
        let candidate =
            TlsProbeCandidateFactory::new(context, path.clone(), TlsProvider::BoringSsl, resolver)
                .from_symbols(&symbols, evidence)
                .map_err(|error| DetectionError::new(path, error.to_string()))?;
        Ok(DetectionOutcome::Matched(candidate))
    }

    pub(in crate::probe_detector::detector::tls::boringssl) fn static_pattern(
        context: &ProbeContext<'_>,
        path: DetectorPath,
        detection: StaticPatternDetection,
        resolver: &'static str,
    ) -> DetectionOutcome {
        let mut evidence = DetectionEvidence::new(path.clone(), detection.arch_label);
        evidence.patterns = detection
            .matches
            .iter()
            .map(|pattern| PatternEvidence {
                pattern_id: pattern.pattern_id.to_string(),
                symbol: pattern.symbol.to_string(),
                pattern_length: pattern.pattern_length,
                match_count: pattern.match_count,
                shown_matches: pattern
                    .shown_matches
                    .iter()
                    .map(|found| EvidenceLocation {
                        file_offset: found.file_offset as u64,
                        virtual_address: found.virtual_address,
                    })
                    .collect(),
            })
            .collect();
        let offsets = detection
            .offsets
            .into_iter()
            .filter(|offset| Self::is_payload_symbol(offset.symbol))
            .map(|offset| {
                (
                    offset.symbol.to_string(),
                    offset.virtual_address,
                    offset.file_offset as u64,
                )
            });
        DetectionOutcome::Matched(
            TlsProbeCandidateFactory::new(context, path, TlsProvider::BoringSsl, resolver)
                .from_offsets(offsets, evidence),
        )
    }

    fn is_payload_symbol(symbol: &str) -> bool {
        matches!(symbol, "SSL_read" | "SSL_write")
    }
}
