use std::collections::HashMap;
use std::ops::Range;
use std::sync::{Arc, OnceLock};

/// The immutable response is authoritative. All derived data shares its lifetime
/// and the fixed matcher configuration of its projector.
#[derive(Clone)]
pub(in crate::llm_pipeline::projection::compact) struct ResponseTextWitness {
    pub(in crate::llm_pipeline::projection::compact) content: Arc<ResponseContent>,
}

pub(in crate::llm_pipeline::projection::compact) struct ResponseContent {
    text: String,
    normalized: Range<usize>,
    chars: OnceLock<usize>,
    pub(in crate::llm_pipeline::projection::compact) json: OnceLock<Option<JsonTextArtifact>>,
    pub(in crate::llm_pipeline::projection::compact) fingerprints: OnceLock<ResponseFingerprints>,
}

pub(in crate::llm_pipeline::projection::compact) struct JsonTextArtifact {
    pub(in crate::llm_pipeline::projection::compact) values: Vec<String>,
    pub(in crate::llm_pipeline::projection::compact) bytes: usize,
}

pub(in crate::llm_pipeline::projection::compact) struct ResponseFingerprints {
    pub(in crate::llm_pipeline::projection::compact) window: usize,
    pub(in crate::llm_pipeline::projection::compact) max_verified_hits: usize,
    pub(in crate::llm_pipeline::projection::compact) positions: HashMap<u64, Vec<usize>>,
}

impl ResponseTextWitness {
    pub(in crate::llm_pipeline::projection::compact) fn new(text: String) -> Option<Self> {
        let without_leading = text.trim_start();
        let start = text.len() - without_leading.len();
        let end = start + without_leading.trim_end().len();
        if start == end {
            return None;
        }
        Some(Self {
            content: Arc::new(ResponseContent {
                text,
                normalized: start..end,
                chars: OnceLock::new(),
                json: OnceLock::new(),
                fingerprints: OnceLock::new(),
            }),
        })
    }

    pub(in crate::llm_pipeline::projection::compact) fn text(&self) -> &str {
        &self.content.text[self.content.normalized.start..self.content.normalized.end]
    }

    pub(in crate::llm_pipeline::projection::compact) fn byte_len(&self) -> usize {
        self.text().len()
    }

    pub(in crate::llm_pipeline::projection::compact) fn char_len(&self) -> usize {
        *self
            .content
            .chars
            .get_or_init(|| self.text().chars().count())
    }
}

pub(in crate::llm_pipeline::projection::compact) enum ResponseWitnessObservation {
    Absent,
    Present(ResponseTextWitness),
    LimitExceeded { bytes: usize },
}
