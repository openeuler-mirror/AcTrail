use crate::probe_detector::contract::detection::DetectionOutcome;
use crate::probe_detector::contract::identity::DetectorPath;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EvidenceFact {
    pub(crate) key: String,
    pub(crate) value: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EvidenceLocation {
    pub(crate) file_offset: u64,
    pub(crate) virtual_address: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PatternEvidence {
    pub(crate) pattern_id: String,
    pub(crate) symbol: String,
    pub(crate) pattern_length: usize,
    pub(crate) match_count: usize,
    pub(crate) shown_matches: Vec<EvidenceLocation>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SymbolEvidence {
    pub(crate) symbol: String,
    pub(crate) runtime_symbol: String,
    pub(crate) virtual_address: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DetectionEvidence {
    pub(crate) detector_path: DetectorPath,
    pub(crate) architecture: String,
    pub(crate) facts: Vec<EvidenceFact>,
    pub(crate) patterns: Vec<PatternEvidence>,
    pub(crate) symbols: Vec<SymbolEvidence>,
    pub(crate) rejection: Option<String>,
    pub(crate) children: Vec<DetectionOutcome>,
}

impl DetectionEvidence {
    pub(crate) fn new(detector_path: DetectorPath, architecture: impl Into<String>) -> Self {
        Self {
            detector_path,
            architecture: architecture.into(),
            facts: Vec::new(),
            patterns: Vec::new(),
            symbols: Vec::new(),
            rejection: None,
            children: Vec::new(),
        }
    }

    pub(crate) fn rejected(mut self, reason: impl Into<String>) -> Self {
        self.rejection = Some(reason.into());
        self
    }
}
