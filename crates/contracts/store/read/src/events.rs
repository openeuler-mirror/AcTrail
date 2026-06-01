//! Event-read contracts.

use model_core::event::DomainEvent;
use model_core::ids::TraceId;

use crate::ReadError;

pub trait EventReadStore {
    fn list_events(&self, trace_id: TraceId) -> Result<Vec<DomainEvent>, ReadError>;
}
