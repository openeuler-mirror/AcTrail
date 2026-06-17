use model_core::event::{DomainEvent, EventPayload};

pub(super) fn event_fd(event: &DomainEvent) -> Option<u32> {
    let EventPayload::File(payload) = &event.payload else {
        return None;
    };
    payload
        .metadata
        .get("fd")
        .and_then(|value| value.parse::<u32>().ok())
}

pub(super) fn event_result(event: &DomainEvent) -> Option<i32> {
    let EventPayload::File(payload) = &event.payload else {
        return None;
    };
    payload.result
}

pub(super) fn event_size(event: &DomainEvent) -> Option<u64> {
    let EventPayload::File(payload) = &event.payload else {
        return None;
    };
    payload
        .metadata
        .get("size")
        .and_then(|value| value.parse::<u64>().ok())
}
