use std::collections::BTreeMap;

use crate::elf::ElfImage;
use crate::plan::{ProbeSource, TlsProvider};
use crate::probe_detector::contract::candidate::verification::VerifiedTarget;
use crate::probe_detector::contract::detection::{
    DetectionEvidence, DetectionOutcome, EvidenceFact, EvidenceLocation, PatternEvidence,
    ProbeContext,
};
use crate::probe_detector::contract::identity::{DetectorId, DetectorPath};
use crate::probe_detector::detector::tls::candidate::TlsProbeCandidateFactory;
use crate::probe_detector::detector::tls::rustls::RUNTIME_SYMBOLS;
use crate::{ToolError, ToolResult};

pub(crate) struct StaticPatternSpec {
    pub(crate) pattern_id: &'static str,
    pub(crate) symbol: &'static str,
    pub(crate) bytes: &'static [u8],
}

pub(crate) struct PatternPairProbeDetector {
    path: DetectorPath,
    candidate_id: &'static str,
    arch_label: &'static str,
    patterns: [StaticPatternSpec; 2],
    verified_targets: Vec<VerifiedTarget>,
}

pub(crate) struct StaticPatternDetection {
    pub(crate) arch_label: &'static str,
    pub(crate) candidate_id: &'static str,
    pub(crate) matches: Vec<PatternMatches>,
    pub(crate) offsets: Vec<DetectedOffset>,
    pub(crate) verified_targets: Vec<VerifiedTarget>,
}

pub(crate) struct PatternMatches {
    pub(crate) pattern_id: &'static str,
    pub(crate) symbol: &'static str,
    pub(crate) pattern_length: usize,
    pub(crate) match_count: usize,
    pub(crate) shown_matches: Vec<OffsetAddress>,
}

pub(crate) struct DetectedOffset {
    pub(crate) symbol: &'static str,
    pub(crate) file_offset: usize,
    pub(crate) virtual_address: u64,
}

pub(crate) struct OffsetAddress {
    pub(crate) file_offset: usize,
    pub(crate) virtual_address: u64,
}

impl PatternPairProbeDetector {
    pub(crate) fn new(
        candidate_id: &'static str,
        arch_label: &'static str,
        patterns: [StaticPatternSpec; 2],
        verified_targets: Vec<VerifiedTarget>,
    ) -> Self {
        let candidate_segment = candidate_id
            .split_once("rustls-")
            .map(|(_, segment)| segment)
            .unwrap_or(candidate_id);
        Self {
            path: DetectorPath::root(DetectorId::new("tls"))
                .child(DetectorId::new("rustls"))
                .child(DetectorId::new("static-pattern"))
                .child(DetectorId::new(arch_label))
                .child(DetectorId::new(candidate_segment)),
            candidate_id,
            arch_label,
            patterns,
            verified_targets,
        }
    }

    pub(crate) fn register_executable_patterns(&self, image: &ElfImage) {
        for pattern in &self.patterns {
            image.register_pattern_scan(pattern.bytes);
        }
    }

    pub(crate) fn detect_outcome(
        &self,
        context: &ProbeContext<'_>,
        match_limit: usize,
    ) -> DetectionOutcome {
        if context.probe.source != ProbeSource::Executable
            || context.probe.image.arch().as_str() != self.arch_label
            || context
                .request
                .requested_provider
                .is_some_and(|provider| provider != TlsProvider::Rustls)
        {
            return DetectionOutcome::Inapplicable(
                DetectionEvidence::new(self.path.clone(), context.target.architecture.clone())
                    .rejected("Rustls static-pattern candidate excluded by context"),
            );
        }
        let detection = match self.detect(context.probe.image, match_limit) {
            Ok(detection) => detection,
            Err(error) => {
                return DetectionOutcome::NoMatch(
                    DetectionEvidence::new(self.path.clone(), context.target.architecture.clone())
                        .rejected(error.to_string()),
                );
            }
        };
        let mut evidence = DetectionEvidence::new(self.path.clone(), detection.arch_label);
        evidence.facts.push(EvidenceFact {
            key: "candidate_id".to_string(),
            value: detection.candidate_id.to_string(),
        });
        for verified in &detection.verified_targets {
            let identity_status = match &verified.identity {
                Some(identity) if identity == context.probe.image.identity() => "exact-match",
                Some(_) => "different-binary",
                None => "not-recorded",
            };
            evidence.facts.extend([
                EvidenceFact {
                    key: "verified_runtime".to_string(),
                    value: verified.runtime_version.to_string(),
                },
                EvidenceFact {
                    key: "verified_compiler_shape".to_string(),
                    value: verified.compiler_shape.to_string(),
                },
                EvidenceFact {
                    key: "verified_identity_status".to_string(),
                    value: identity_status.to_string(),
                },
                EvidenceFact {
                    key: "verified_evidence_source".to_string(),
                    value: verified.evidence_source.to_string(),
                },
            ]);
        }
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
        let offsets = detection.offsets.into_iter().map(|offset| {
            (
                offset.symbol.to_string(),
                offset.virtual_address,
                offset.file_offset as u64,
            )
        });
        DetectionOutcome::Matched(
            TlsProbeCandidateFactory::new(
                context,
                self.path.clone(),
                TlsProvider::Rustls,
                crate::probe_detector::detector::tls::rustls::RESOLVER,
            )
            .from_offsets(offsets, evidence),
        )
    }

    fn detect(&self, image: &ElfImage, match_limit: usize) -> ToolResult<StaticPatternDetection> {
        let executable_ranges = image.executable_file_ranges()?;
        let mut matches = Vec::new();
        let mut offsets_by_symbol = BTreeMap::<&'static str, Vec<usize>>::new();
        let pattern_bytes = self
            .patterns
            .iter()
            .map(|pattern| pattern.bytes)
            .collect::<Vec<_>>();
        let all_offsets = image.pattern_offsets_for(&pattern_bytes, &executable_ranges);
        for (pattern, pattern_offsets) in self.patterns.iter().zip(all_offsets) {
            offsets_by_symbol
                .entry(pattern.symbol)
                .or_default()
                .extend(pattern_offsets.iter().copied());
            matches.push(Self::pattern_matches(
                image,
                pattern,
                &pattern_offsets,
                match_limit,
            )?);
        }
        let required_offsets = RUNTIME_SYMBOLS
            .iter()
            .map(|symbol| {
                let offsets = offsets_by_symbol.remove(symbol).unwrap_or_default();
                Ok((*symbol, Self::require_single_unique(&offsets, symbol)?))
            })
            .collect::<ToolResult<Vec<_>>>()?;
        let offsets = Self::offsets_with_addresses(image, &required_offsets)?;
        Ok(StaticPatternDetection {
            arch_label: self.arch_label,
            candidate_id: self.candidate_id,
            matches,
            offsets,
            verified_targets: self.verified_targets.clone(),
        })
    }

    fn require_single_unique(matches: &[usize], symbol: &str) -> ToolResult<usize> {
        let mut unique = matches.to_vec();
        unique.sort_unstable();
        unique.dedup();
        if unique.len() == 1 {
            Ok(unique[0])
        } else {
            Err(ToolError::new(format!(
                "rustls {symbol} pattern match count={}",
                unique.len()
            )))
        }
    }

    fn offsets_with_addresses(
        image: &ElfImage,
        offsets: &[(&'static str, usize)],
    ) -> ToolResult<Vec<DetectedOffset>> {
        offsets
            .iter()
            .map(|(symbol, file_offset)| {
                let virtual_address = image.virtual_address_for_file_offset(*file_offset as u64)?;
                Ok(DetectedOffset {
                    symbol,
                    file_offset: *file_offset,
                    virtual_address,
                })
            })
            .collect()
    }

    fn pattern_matches(
        image: &ElfImage,
        pattern: &StaticPatternSpec,
        matches: &[usize],
        match_limit: usize,
    ) -> ToolResult<PatternMatches> {
        let shown_matches = matches
            .iter()
            .take(match_limit)
            .map(|file_offset| {
                let virtual_address = image.virtual_address_for_file_offset(*file_offset as u64)?;
                Ok(OffsetAddress {
                    file_offset: *file_offset,
                    virtual_address,
                })
            })
            .collect::<ToolResult<Vec<_>>>()?;
        Ok(PatternMatches {
            pattern_id: pattern.pattern_id,
            symbol: pattern.symbol,
            pattern_length: pattern.bytes.len(),
            match_count: matches.len(),
            shown_matches,
        })
    }
}
