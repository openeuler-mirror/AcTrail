//! Detection report data returned by provider resolution.

use crate::elf::ElfImage;

pub(crate) struct DetectReport {
    pub(crate) target: TargetReport,
    pub(crate) notices: Vec<String>,
    pub(crate) candidates: Vec<CandidateReport>,
}

impl DetectReport {
    pub(crate) fn success(&self) -> bool {
        self.candidates
            .iter()
            .any(|candidate| matches!(candidate.status, CandidateStatus::Matched))
    }

    pub(crate) fn failure_message(&self) -> String {
        let failures = self
            .candidates
            .iter()
            .filter_map(|candidate| match &candidate.status {
                CandidateStatus::Failed { error } => Some(format!(
                    "{}/{}: {}",
                    candidate.source, candidate.provider, error
                )),
                CandidateStatus::Matched | CandidateStatus::Available => None,
            })
            .collect::<Vec<_>>();
        if failures.is_empty() {
            "no TLS provider matched".to_string()
        } else {
            format!("no TLS provider matched: {}", failures.join("; "))
        }
    }
}

pub(crate) struct TargetReport {
    pub(crate) binary: String,
    pub(crate) architecture: String,
    pub(crate) build_id: String,
}

impl TargetReport {
    pub(crate) fn from_image(image: &ElfImage) -> Self {
        Self {
            binary: image.path().display().to_string(),
            architecture: image.arch().as_str().to_string(),
            build_id: image.build_id().unwrap_or("not_found").to_string(),
        }
    }
}

pub(crate) struct CandidateReport {
    pub(crate) source: String,
    pub(crate) provider: String,
    pub(crate) library: Option<LibraryReport>,
    pub(crate) exported_symbols: Vec<ExportedSymbolReport>,
    pub(crate) exported_symbol_map: Option<MapStatusReport>,
    pub(crate) demangled_symbols: Option<DemangledSymbolReport>,
    pub(crate) endpoints: Vec<EndpointReport>,
    pub(crate) endpoint_status: Option<MapStatusReport>,
    pub(crate) pattern_matches: Option<PatternMatchesReport>,
    pub(crate) detected_offsets: Vec<DetectedOffsetReport>,
    pub(crate) runtime_config: Vec<(String, String)>,
    pub(crate) symbol_map: Option<SymbolMapReport>,
    pub(crate) status: CandidateStatus,
}

impl CandidateReport {
    pub(crate) fn new(source: &str, provider: &str) -> Self {
        Self {
            source: source.to_string(),
            provider: provider.to_string(),
            library: None,
            exported_symbols: Vec::new(),
            exported_symbol_map: None,
            demangled_symbols: None,
            endpoints: Vec::new(),
            endpoint_status: None,
            pattern_matches: None,
            detected_offsets: Vec::new(),
            runtime_config: Vec::new(),
            symbol_map: None,
            status: CandidateStatus::Failed {
                error: "candidate not evaluated".to_string(),
            },
        }
    }

    pub(crate) fn failed(source: &str, provider: &str, error: impl Into<String>) -> Self {
        let mut candidate = Self::new(source, provider);
        candidate.status = CandidateStatus::Failed {
            error: error.into(),
        };
        candidate
    }
}

pub(crate) struct LibraryReport {
    pub(crate) path: String,
    pub(crate) confidence: String,
    pub(crate) note: Option<String>,
    pub(crate) architecture: Option<String>,
    pub(crate) build_id: Option<String>,
}

pub(crate) struct ExportedSymbolReport {
    pub(crate) name: String,
    pub(crate) entries: Vec<ExportedSymbolEntry>,
}

pub(crate) struct ExportedSymbolEntry {
    pub(crate) value: String,
    pub(crate) size: String,
    pub(crate) bind: String,
    pub(crate) ndx: String,
    pub(crate) table: String,
    pub(crate) raw: String,
}

pub(crate) struct MapStatusReport {
    pub(crate) status: String,
    pub(crate) fields: Vec<(String, String)>,
}

impl MapStatusReport {
    pub(crate) fn missing(status: &str, missing: &[String]) -> Self {
        Self {
            status: status.to_string(),
            fields: vec![("missing".to_string(), missing.join(","))],
        }
    }
}

pub(crate) struct DemangledSymbolReport {
    pub(crate) status: String,
    pub(crate) source: String,
    pub(crate) binary: String,
    pub(crate) targets: Vec<DemangledSymbolTargetReport>,
}

pub(crate) struct DemangledSymbolTargetReport {
    pub(crate) symbol: String,
    pub(crate) address: String,
    pub(crate) runtime_symbol: String,
}

pub(crate) struct EndpointReport {
    pub(crate) symbol: String,
    pub(crate) virtual_address: String,
    pub(crate) file_offset: String,
}

pub(crate) struct PatternMatchesReport {
    pub(crate) arch: String,
    pub(crate) entries: Vec<PatternMatchReport>,
}

pub(crate) struct PatternMatchReport {
    pub(crate) pattern_id: String,
    pub(crate) symbol: String,
    pub(crate) library: String,
    pub(crate) resolver: String,
    pub(crate) pattern_length: String,
    pub(crate) match_count: usize,
    pub(crate) matches: Vec<OffsetAddressReport>,
}

pub(crate) struct OffsetAddressReport {
    pub(crate) file_offset: String,
    pub(crate) virtual_address: String,
}

pub(crate) struct DetectedOffsetReport {
    pub(crate) symbol: String,
    pub(crate) file_offset: String,
    pub(crate) virtual_address: String,
}

pub(crate) struct SymbolMapReport {
    pub(crate) resolver: String,
    pub(crate) library: String,
    pub(crate) arch: String,
    pub(crate) build_id: String,
    pub(crate) symbols: Vec<(String, String)>,
}

pub(crate) enum CandidateStatus {
    Matched,
    Available,
    Failed { error: String },
}
