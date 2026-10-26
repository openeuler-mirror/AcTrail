//! Schema-v35 payload-segment state word.
//!
//! Bit 0 is direction, bit 1 content state, bits 2..3 source boundary,
//! bits 4..5 operation completion, bits 6..7 truncation, and bits 8..9
//! redaction. Codes and bit positions are persistent ABI: changing them
//! requires a SQLite schema-version bump.

use model_core::payload::{
    PayloadContentState, PayloadDirection, PayloadOperationCompletionState, PayloadRedactionState,
    PayloadSegment, PayloadSourceBoundary, PayloadTruncationState,
};
use rusqlite::Error as SqlError;

const DIRECTION_SHIFT: u32 = 0;
const CONTENT_SHIFT: u32 = 1;
const SOURCE_SHIFT: u32 = 2;
const COMPLETION_SHIFT: u32 = 4;
const TRUNCATION_SHIFT: u32 = 6;
const REDACTION_SHIFT: u32 = 8;
const ONE_BIT_MASK: u16 = 0b1;
const TWO_BIT_MASK: u16 = 0b11;
const KNOWN_BITS_MASK: u16 = 0x03ff;
pub(crate) const PAYLOAD_DIRECTION_MASK: i64 = 1;

#[derive(Clone, Copy)]
pub(crate) struct PayloadSegmentMeta(u16);

impl PayloadSegmentMeta {
    pub(crate) fn from_segment(segment: &PayloadSegment) -> Self {
        let direction = match segment.direction {
            PayloadDirection::Outbound => 0,
            PayloadDirection::Inbound => 1,
        };
        let content = match segment.content_state {
            PayloadContentState::Plaintext => 0,
            PayloadContentState::Ciphertext => 1,
        };
        let source = match segment.source_boundary {
            PayloadSourceBoundary::TlsUserSpace => 0,
            PayloadSourceBoundary::Syscall => 1,
            PayloadSourceBoundary::Stdio => 2,
        };
        let completion = match segment.operation_completion_state {
            PayloadOperationCompletionState::Unknown => 0,
            PayloadOperationCompletionState::Success => 1,
            PayloadOperationCompletionState::Partial => 2,
            PayloadOperationCompletionState::Failed => 3,
        };
        let truncation = match segment.truncation {
            PayloadTruncationState::Complete => 0,
            PayloadTruncationState::Truncated => 1,
            PayloadTruncationState::PolicyLimited => 2,
        };
        let redaction = match segment.redaction {
            PayloadRedactionState::NotRequired => 0,
            PayloadRedactionState::Redacted => 1,
            PayloadRedactionState::Unredacted => 2,
        };
        Self(
            (direction << DIRECTION_SHIFT)
                | (content << CONTENT_SHIFT)
                | (source << SOURCE_SHIFT)
                | (completion << COMPLETION_SHIFT)
                | (truncation << TRUNCATION_SHIFT)
                | (redaction << REDACTION_SHIFT),
        )
    }

    pub(crate) fn from_code(code: i64) -> Result<Self, SqlError> {
        let code = u16::try_from(code).map_err(|_| SqlError::InvalidQuery)?;
        if code & !KNOWN_BITS_MASK != 0
            || Self::field(code, SOURCE_SHIFT) == 3
            || Self::field(code, TRUNCATION_SHIFT) == 3
            || Self::field(code, REDACTION_SHIFT) == 3
        {
            return Err(SqlError::InvalidQuery);
        }
        Ok(Self(code))
    }

    pub(crate) const fn code(self) -> i64 {
        self.0 as i64
    }

    pub(crate) const fn direction_bits(direction: PayloadDirection) -> i64 {
        match direction {
            PayloadDirection::Outbound => 0,
            PayloadDirection::Inbound => 1,
        }
    }

    pub(crate) fn source_boundary(self) -> Result<PayloadSourceBoundary, SqlError> {
        match Self::field(self.0, SOURCE_SHIFT) {
            0 => Ok(PayloadSourceBoundary::TlsUserSpace),
            1 => Ok(PayloadSourceBoundary::Syscall),
            2 => Ok(PayloadSourceBoundary::Stdio),
            _ => Err(SqlError::InvalidQuery),
        }
    }

    pub(crate) fn content_state(self) -> Result<PayloadContentState, SqlError> {
        match Self::bit(self.0, CONTENT_SHIFT) {
            0 => Ok(PayloadContentState::Plaintext),
            1 => Ok(PayloadContentState::Ciphertext),
            _ => Err(SqlError::InvalidQuery),
        }
    }

    pub(crate) fn direction(self) -> Result<PayloadDirection, SqlError> {
        match Self::bit(self.0, DIRECTION_SHIFT) {
            0 => Ok(PayloadDirection::Outbound),
            1 => Ok(PayloadDirection::Inbound),
            _ => Err(SqlError::InvalidQuery),
        }
    }

    pub(crate) fn operation_completion_state(
        self,
    ) -> Result<PayloadOperationCompletionState, SqlError> {
        match Self::field(self.0, COMPLETION_SHIFT) {
            0 => Ok(PayloadOperationCompletionState::Unknown),
            1 => Ok(PayloadOperationCompletionState::Success),
            2 => Ok(PayloadOperationCompletionState::Partial),
            3 => Ok(PayloadOperationCompletionState::Failed),
            _ => Err(SqlError::InvalidQuery),
        }
    }

    pub(crate) fn truncation(self) -> Result<PayloadTruncationState, SqlError> {
        match Self::field(self.0, TRUNCATION_SHIFT) {
            0 => Ok(PayloadTruncationState::Complete),
            1 => Ok(PayloadTruncationState::Truncated),
            2 => Ok(PayloadTruncationState::PolicyLimited),
            _ => Err(SqlError::InvalidQuery),
        }
    }

    pub(crate) fn redaction(self) -> Result<PayloadRedactionState, SqlError> {
        match Self::field(self.0, REDACTION_SHIFT) {
            0 => Ok(PayloadRedactionState::NotRequired),
            1 => Ok(PayloadRedactionState::Redacted),
            2 => Ok(PayloadRedactionState::Unredacted),
            _ => Err(SqlError::InvalidQuery),
        }
    }

    const fn field(code: u16, shift: u32) -> u16 {
        (code >> shift) & TWO_BIT_MASK
    }

    const fn bit(code: u16, shift: u32) -> u16 {
        (code >> shift) & ONE_BIT_MASK
    }
}
