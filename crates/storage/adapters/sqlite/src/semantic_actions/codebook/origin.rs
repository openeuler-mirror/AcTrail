use semantic_action::SemanticActionLinkOrigin;

use super::CodebookError;

#[derive(Clone, Copy)]
pub(crate) struct LinkOriginCodes {
    pub(crate) observed: i16,
    pub(crate) derived: i16,
}

impl LinkOriginCodes {
    pub(crate) const fn code(self, value: SemanticActionLinkOrigin) -> i16 {
        match value {
            SemanticActionLinkOrigin::Observed => self.observed,
            SemanticActionLinkOrigin::Derived => self.derived,
        }
    }

    pub(crate) fn decode(self, code: i64) -> Result<SemanticActionLinkOrigin, CodebookError> {
        let code = i16::try_from(code)
            .map_err(|_| CodebookError::unknown("semantic_action_link_origin_code", code))?;
        match code {
            value if value == self.observed => Ok(SemanticActionLinkOrigin::Observed),
            value if value == self.derived => Ok(SemanticActionLinkOrigin::Derived),
            _ => Err(CodebookError::unknown(
                "semantic_action_link_origin_code",
                code,
            )),
        }
    }

    pub(super) fn entries(self) -> [(&'static str, i16); 2] {
        [
            (SemanticActionLinkOrigin::Observed.as_str(), self.observed),
            (SemanticActionLinkOrigin::Derived.as_str(), self.derived),
        ]
    }
}
