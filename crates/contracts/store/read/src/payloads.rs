//! Payload-read contracts.

use model_core::ids::TraceId;
use model_core::payload::{PayloadDirection, PayloadSegment, PayloadSegmentId};

use crate::ReadError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PayloadRowLimit {
    Head(usize),
    Tail(usize),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PayloadSegmentQuery {
    pub segment_id: Option<PayloadSegmentId>,
    pub direction: Option<PayloadDirection>,
    pub limit: Option<PayloadRowLimit>,
    pub include_bytes: bool,
}

impl PayloadSegmentQuery {
    pub const fn metadata_only(limit: Option<PayloadRowLimit>) -> Self {
        Self {
            segment_id: None,
            direction: None,
            limit,
            include_bytes: false,
        }
    }
}

pub trait PayloadReadStore {
    fn list_payload_segments(
        &self,
        trace_id: TraceId,
        query: PayloadSegmentQuery,
    ) -> Result<Vec<PayloadSegment>, ReadError>;

    fn retained_payload_bytes(&self, trace_id: TraceId) -> Result<u64, ReadError>;
}
