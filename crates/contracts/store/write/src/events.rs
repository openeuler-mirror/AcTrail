//! Event-write contracts.

use model_core::event::DomainEvent;

use crate::WriteError;

pub trait EventWriteStore {
    fn append_event(&mut self, event: DomainEvent) -> Result<(), WriteError>;
}
