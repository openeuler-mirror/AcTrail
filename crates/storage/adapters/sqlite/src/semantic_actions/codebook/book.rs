use super::error::validate_unique;
use super::{ActionKindCodes, CodebookError, LinkOriginCodes, LinkRoleCodes};
use semantic_action::{SemanticActionCompleteness, SemanticActionStatus, SemanticEvidenceKind};

pub(crate) const CURRENT_SCHEMA_VERSION: i32 = super::current::SCHEMA_VERSION;

pub(crate) fn current() -> &'static SemanticActionCodebook {
    super::current::CODEBOOK
}

pub(crate) fn for_schema_version(
    schema_version: i32,
) -> Result<&'static SemanticActionCodebook, CodebookError> {
    match schema_version {
        CURRENT_SCHEMA_VERSION => Ok(super::current::CODEBOOK),
        _ => Err(CodebookError::new(
            "semantic_action_schema_version",
            format!("unsupported semantic action codebook schema version {schema_version}"),
        )),
    }
}

#[derive(Clone, Copy)]
pub(crate) struct SemanticActionCodebook {
    pub(crate) action_kind: ActionKindCodes,
    pub(crate) action_status: ActionStatusCodes,
    pub(crate) action_completeness: ActionCompletenessCodes,
    pub(crate) evidence_kind: EvidenceKindCodes,
    pub(crate) link_role: LinkRoleCodes,
    pub(crate) link_origin: LinkOriginCodes,
}

impl SemanticActionCodebook {
    pub(crate) fn validate(self) -> Result<(), CodebookError> {
        validate_unique("semantic_action_kind", &self.action_kind.entries())?;
        validate_unique("semantic_action_status", &self.action_status.entries())?;
        validate_unique(
            "semantic_action_completeness",
            &self.action_completeness.entries(),
        )?;
        validate_unique("semantic_evidence_kind", &self.evidence_kind.entries())?;
        validate_unique("semantic_action_link_role", &self.link_role.entries())?;
        validate_unique("semantic_action_link_origin", &self.link_origin.entries())
    }
}

#[derive(Clone, Copy)]
pub(crate) struct ActionStatusCodes {
    pub(crate) in_progress: i16,
    pub(crate) success: i16,
    pub(crate) error: i16,
    pub(crate) unknown: i16,
}

impl ActionStatusCodes {
    pub(crate) const fn code(self, value: SemanticActionStatus) -> i16 {
        match value {
            SemanticActionStatus::InProgress => self.in_progress,
            SemanticActionStatus::Success => self.success,
            SemanticActionStatus::Error => self.error,
            SemanticActionStatus::Unknown => self.unknown,
        }
    }

    pub(crate) fn decode(self, code: i64) -> Result<SemanticActionStatus, CodebookError> {
        let code = i16::try_from(code)
            .map_err(|_| CodebookError::unknown("semantic_action_status_code", code))?;
        match code {
            value if value == self.in_progress => Ok(SemanticActionStatus::InProgress),
            value if value == self.success => Ok(SemanticActionStatus::Success),
            value if value == self.error => Ok(SemanticActionStatus::Error),
            value if value == self.unknown => Ok(SemanticActionStatus::Unknown),
            _ => Err(CodebookError::unknown("semantic_action_status_code", code)),
        }
    }

    fn entries(self) -> [(&'static str, i16); 4] {
        [
            (SemanticActionStatus::InProgress.as_str(), self.in_progress),
            (SemanticActionStatus::Success.as_str(), self.success),
            (SemanticActionStatus::Error.as_str(), self.error),
            (SemanticActionStatus::Unknown.as_str(), self.unknown),
        ]
    }
}

#[derive(Clone, Copy)]
pub(crate) struct ActionCompletenessCodes {
    pub(crate) complete: i16,
    pub(crate) partial: i16,
    pub(crate) inferred: i16,
    pub(crate) capture_limited: i16,
}

impl ActionCompletenessCodes {
    pub(crate) const fn code(self, value: SemanticActionCompleteness) -> i16 {
        match value {
            SemanticActionCompleteness::Complete => self.complete,
            SemanticActionCompleteness::Partial => self.partial,
            SemanticActionCompleteness::CaptureLimited => self.capture_limited,
            SemanticActionCompleteness::Inferred => self.inferred,
        }
    }

    pub(crate) fn decode(self, code: i64) -> Result<SemanticActionCompleteness, CodebookError> {
        let code = i16::try_from(code)
            .map_err(|_| CodebookError::unknown("semantic_action_completeness_code", code))?;
        match code {
            value if value == self.complete => Ok(SemanticActionCompleteness::Complete),
            value if value == self.partial => Ok(SemanticActionCompleteness::Partial),
            value if value == self.capture_limited => {
                Ok(SemanticActionCompleteness::CaptureLimited)
            }
            value if value == self.inferred => Ok(SemanticActionCompleteness::Inferred),
            _ => Err(CodebookError::unknown(
                "semantic_action_completeness_code",
                code,
            )),
        }
    }

    fn entries(self) -> [(&'static str, i16); 4] {
        [
            (SemanticActionCompleteness::Complete.as_str(), self.complete),
            (SemanticActionCompleteness::Partial.as_str(), self.partial),
            (
                SemanticActionCompleteness::CaptureLimited.as_str(),
                self.capture_limited,
            ),
            (SemanticActionCompleteness::Inferred.as_str(), self.inferred),
        ]
    }
}

#[derive(Clone, Copy)]
pub(crate) struct EvidenceKindCodes {
    pub(crate) event: i16,
    pub(crate) payload_aggregate: i16,
    pub(crate) payload_segment: i16,
}

impl EvidenceKindCodes {
    pub(crate) const fn code(self, value: SemanticEvidenceKind) -> i16 {
        match value {
            SemanticEvidenceKind::Event => self.event,
            SemanticEvidenceKind::PayloadAggregate => self.payload_aggregate,
            SemanticEvidenceKind::PayloadSegment => self.payload_segment,
        }
    }

    pub(crate) fn decode(self, code: i64) -> Result<SemanticEvidenceKind, CodebookError> {
        let code = i16::try_from(code)
            .map_err(|_| CodebookError::unknown("semantic_evidence_kind_code", code))?;
        match code {
            value if value == self.event => Ok(SemanticEvidenceKind::Event),
            value if value == self.payload_aggregate => Ok(SemanticEvidenceKind::PayloadAggregate),
            value if value == self.payload_segment => Ok(SemanticEvidenceKind::PayloadSegment),
            _ => Err(CodebookError::unknown("semantic_evidence_kind_code", code)),
        }
    }

    fn entries(self) -> [(&'static str, i16); 3] {
        [
            (SemanticEvidenceKind::Event.as_str(), self.event),
            (
                SemanticEvidenceKind::PayloadAggregate.as_str(),
                self.payload_aggregate,
            ),
            (
                SemanticEvidenceKind::PayloadSegment.as_str(),
                self.payload_segment,
            ),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use semantic_action::{SemanticActionKind, SemanticActionLinkRole};

    #[test]
    fn tool_interaction_codes_are_unique_and_round_trip() {
        let codebook = current();
        codebook.validate().expect("current codebook is unique");
        for kind in [
            SemanticActionKind::LlmToolCall,
            SemanticActionKind::LlmToolResult,
        ] {
            let code = codebook.action_kind.code(kind);
            assert_eq!(codebook.action_kind.decode(i64::from(code)), Ok(kind));
        }
        for role in [
            SemanticActionLinkRole::LlmRequestTrajectoryParent,
            SemanticActionLinkRole::LlmRequestTrajectoryFork,
            SemanticActionLinkRole::LlmResponseToolCall,
            SemanticActionLinkRole::LlmToolCallResult,
            SemanticActionLinkRole::LlmToolCallAgentInvocation,
        ] {
            let code = codebook.link_role.code(role);
            assert_eq!(codebook.link_role.decode(i64::from(code)), Ok(role));
        }
    }
}
