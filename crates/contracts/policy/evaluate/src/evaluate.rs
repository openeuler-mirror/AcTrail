//! Host-facing policy evaluation contracts.

use std::collections::BTreeMap;

use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;

use crate::decision::PolicyDecision;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicyInput {
    pub trace_id: TraceId,
    pub process: ProcessIdentity,
    pub event_kind: String,
    pub fields: BTreeMap<String, String>,
    pub bytes: Vec<u8>,
}

pub trait PolicyEvaluator {
    fn evaluate(&self, input: &PolicyInput) -> PolicyDecision;
}
