pub use context::ProbeConsumer;
pub(crate) use context::{DetectionRequest, LibraryCandidate, ProbeContext};
pub(crate) use error::DetectionError;
pub(crate) use evidence::{
    DetectionEvidence, EvidenceFact, EvidenceLocation, PatternEvidence, SymbolEvidence,
};
pub(crate) use outcome::{AmbiguousDetection, DetectionOutcome};

mod context;
mod error;
mod evidence;
mod outcome;
