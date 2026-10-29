//! Explicit changes to an action's runtime-owned state.

use std::collections::BTreeMap;
use std::time::SystemTime;

use model_core::{ids::TraceId, process::ProcessIdentity};

use crate::{
    SemanticActionCompleteness, SemanticActionKind, SemanticActionStatus, SemanticActionStoreError,
    SemanticAttributeSupplementRejection, SemanticEvidence,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticActionUpdate {
    pub action_id: String,
    pub trace_id: TraceId,
    pub kind: SemanticActionKind,
    pub process: ProcessIdentity,
    pub change: SemanticActionChange,
    pub evidence: Vec<SemanticEvidence>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SemanticActionChange {
    /// Replace the producer-owned title and complete attributes, including removals.
    /// Lifecycle facts are assigned independently through `Lifecycle`.
    ActionSnapshot {
        title: String,
        attributes: BTreeMap<String, String>,
    },
    Lifecycle {
        end_time: Option<SystemTime>,
        status: SemanticActionStatus,
        completeness: SemanticActionCompleteness,
        finalization_reason: Option<SemanticActionFinalizationReason>,
    },
    LlmResponseFailure {
        title: String,
        end_time: Option<SystemTime>,
        body_format: String,
        http_status_code: Option<u16>,
        http_reason: Option<String>,
    },
    CommandClassification {
        kind: SemanticCommandKind,
    },
    CommandToolName {
        tool_name: String,
    },
    ToolResultBinding {
        state: SemanticToolResultBinding,
    },
    SupplementAttributes {
        supplement: SemanticAttributeSupplement,
    },
}

/// An immutable related record required before applying an optional supplement.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticActionIdentity {
    pub action_id: String,
    pub trace_id: TraceId,
    pub kind: SemanticActionKind,
    pub process: ProcessIdentity,
}

/// Producer-owned conditions and additions. Existing values are never replaced.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticAttributeSupplement {
    pub expected_attributes: BTreeMap<String, String>,
    pub attributes: BTreeMap<String, String>,
    pub required_action: Option<SemanticActionIdentity>,
}

impl SemanticAttributeSupplement {
    /// Validate all conditions before mutating; identical repetitions are idempotent.
    pub fn merge_attributes(
        &self,
        attributes: &mut BTreeMap<String, String>,
    ) -> Result<bool, SemanticActionStoreError> {
        if self
            .expected_attributes
            .iter()
            .any(|(key, value)| attributes.get(key) != Some(value))
        {
            return Err(SemanticActionStoreError::supplement_rejected(
                SemanticAttributeSupplementRejection::Identity,
                "supplement_attributes",
                "expected action attributes conflict",
            ));
        }
        if self
            .attributes
            .iter()
            .any(|(key, value)| attributes.get(key).is_some_and(|current| current != value))
        {
            return Err(SemanticActionStoreError::supplement_rejected(
                SemanticAttributeSupplementRejection::Conflict,
                "supplement_attributes",
                "supplement attributes conflict",
            ));
        }
        let changed = self
            .attributes
            .keys()
            .any(|key| !attributes.contains_key(key));
        if changed {
            attributes.extend(self.attributes.clone());
        }
        Ok(changed)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SemanticActionFinalizationReason {
    TraceClosed,
    CapacityEvicted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SemanticToolResultBinding {
    MissingId,
    Unmatched,
    Bound,
    Ambiguous,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SemanticCommandKind {
    Agent,
    Mcp,
    Command,
}
