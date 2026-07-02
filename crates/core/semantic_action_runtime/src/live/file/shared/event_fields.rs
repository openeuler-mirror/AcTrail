use model_core::event::{DomainEvent, EventPayload};

pub(in crate::live::file) fn event_fd(event: &DomainEvent) -> Option<u32> {
    let EventPayload::File(payload) = &event.payload else {
        return None;
    };
    payload
        .metadata
        .get("fd")
        .and_then(|value| value.parse::<u32>().ok())
}

pub(in crate::live::file) fn event_source_fd(event: &DomainEvent) -> Option<u32> {
    event_metadata_u32(event, "source_fd")
}

pub(in crate::live::file) fn event_target_fd(event: &DomainEvent) -> Option<u32> {
    event_metadata_u32(event, "target_fd")
}

pub(in crate::live::file) fn event_result(event: &DomainEvent) -> Option<i32> {
    let EventPayload::File(payload) = &event.payload else {
        return None;
    };
    payload.result
}

pub(in crate::live::file) fn event_size(event: &DomainEvent) -> Option<u64> {
    let EventPayload::File(payload) = &event.payload else {
        return None;
    };
    payload
        .metadata
        .get("size")
        .and_then(|value| value.parse::<u64>().ok())
}

pub(in crate::live::file) fn event_read_summary_count(event: &DomainEvent) -> Option<u64> {
    let EventPayload::File(payload) = &event.payload else {
        return None;
    };
    payload
        .metadata
        .get("read_count")
        .and_then(|value| value.parse::<u64>().ok())
}

pub(in crate::live::file) fn event_file_path(event: &DomainEvent) -> Option<String> {
    let EventPayload::File(payload) = &event.payload else {
        return None;
    };
    payload_file_path(payload)
}

pub(in crate::live::file) fn payload_file_path(
    payload: &model_core::event::FilePayload,
) -> Option<String> {
    payload
        .path
        .as_deref()
        .or_else(|| payload.metadata.get("fd_target").map(String::as_str))
        .and_then(canonical_file_path)
}

pub(in crate::live::file) fn file_open_has_directory_flag(
    payload: &model_core::event::FilePayload,
) -> bool {
    let Some(flags) = payload
        .metadata
        .get("flags")
        .and_then(|value| value.parse::<u64>().ok())
    else {
        return false;
    };
    flags & libc::O_DIRECTORY as u64 != 0
}

fn event_metadata_u32(event: &DomainEvent, key: &str) -> Option<u32> {
    let EventPayload::File(payload) = &event.payload else {
        return None;
    };
    payload
        .metadata
        .get(key)
        .and_then(|value| value.parse::<u32>().ok())
}

fn canonical_file_path(path: &str) -> Option<String> {
    if path.is_empty() {
        return None;
    }
    if !path.starts_with('/') {
        return None;
    }
    let mut parts = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                let _ = parts.pop();
            }
            _ => parts.push(part),
        }
    }
    if parts.is_empty() {
        return Some("/".to_string());
    }
    Some(format!("/{}", parts.join("/")))
}
