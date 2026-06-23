//! File event assertions for live eBPF verification.

use std::collections::HashSet;
use std::path::Path;

use model_core::event::{DomainEvent, EventPayload, FilePayload};
use semantic_action::{SemanticAction, attr_keys as attrs};

pub(super) fn observed(events: &[DomainEvent], actions: &[SemanticAction]) -> HashSet<String> {
    let mut observed = events
        .iter()
        .filter_map(|event| match &event.payload {
            EventPayload::File(payload) => Some(payload.operation.clone()),
            _ => None,
        })
        .collect::<HashSet<_>>();
    for action in actions {
        match action.kind.as_str() {
            "file.read" => {
                observed.insert("read".to_string());
            }
            "file.write" => {
                observed.insert("write".to_string());
            }
            "file.bulk_read" => {
                if positive_u64_attribute(action, attrs::file_bulk_read::READ_COUNT) {
                    observed.insert("read".to_string());
                }
            }
            "file.modify" => {
                if let Some(operation) = action
                    .attributes
                    .get(attrs::file::OPERATION)
                    .or_else(|| action.attributes.get("operation"))
                {
                    observed.insert(operation.clone());
                }
            }
            _ => {}
        }
    }
    observed
}

pub(super) fn require(
    events: &[DomainEvent],
    actions: &[SemanticAction],
    expected_file_path: &Path,
    expected_mmap: Option<(u64, u64)>,
) -> Result<(), String> {
    let mut failures = Vec::new();
    let mut matching_mmap = false;
    for event in events {
        let EventPayload::File(payload) = &event.payload else {
            continue;
        };
        if payload.operation != "close" && payload.path.is_none() && !path_read_fault(payload) {
            failures.push(format!("{} missing path", payload.operation));
        }
        if payload.result.is_none() {
            failures.push(format!("{} missing syscall result", payload.operation));
        }
        match payload.operation.as_str() {
            "read" | "write" => {
                if !payload.metadata.contains_key("fd") {
                    failures.push(format!("{} missing fd", payload.operation));
                }
                if !payload.metadata.contains_key("size") {
                    failures.push(format!("{} missing transferred size", payload.operation));
                }
            }
            "rename" => {
                require_metadata(payload, "target_path", &mut failures);
                require_metadata(payload, "target_path_captured_size", &mut failures);
            }
            "truncate" => {
                if !payload.metadata.contains_key("length")
                    && !payload.metadata.contains_key("truncate_source")
                {
                    failures.push("truncate missing length or truncate_source".to_string());
                }
            }
            "mmap_shared" => {
                if let Some((expected_length, expected_offset)) = expected_mmap {
                    require_mmap_metadata(
                        payload,
                        expected_length,
                        expected_offset,
                        &mut matching_mmap,
                        &mut failures,
                    );
                } else {
                    failures.push("mmap_shared observed while mmap is not configured".to_string());
                }
            }
            "open" | "mkdir" | "rmdir" | "unlink" | "close" => {}
            _ => failures.push(format!("unexpected file operation {}", payload.operation)),
        }
        if matches!(
            payload.operation.as_str(),
            "open" | "mkdir" | "rmdir" | "rename" | "unlink" | "truncate" | "mmap_shared"
        ) {
            require_metadata(payload, "path_captured_size", &mut failures);
            require_metadata(payload, "path_max_bytes", &mut failures);
            require_metadata(payload, "syscall", &mut failures);
        }
    }
    if let Some((expected_mmap_length, expected_mmap_offset)) = expected_mmap
        && !matching_mmap
    {
        failures.push(format!(
            "mmap_shared missing expected length={} offset={} shared=true",
            expected_mmap_length, expected_mmap_offset
        ));
    }
    require_workload_file_io(events, actions, expected_file_path, &mut failures);
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("; "))
    }
}

fn require_mmap_metadata(
    payload: &FilePayload,
    expected_length: u64,
    expected_offset: u64,
    matching_mmap: &mut bool,
    failures: &mut Vec<String>,
) {
    require_metadata(payload, "length", failures);
    require_metadata(payload, "protection", failures);
    require_metadata(payload, "flags", failures);
    require_metadata(payload, "offset", failures);
    require_metadata(payload, "shared", failures);
    require_metadata(payload, "mapped_address", failures);
    if payload
        .metadata
        .get("length")
        .is_some_and(|value| value == &expected_length.to_string())
        && payload
            .metadata
            .get("offset")
            .is_some_and(|value| value == &expected_offset.to_string())
        && payload
            .metadata
            .get("shared")
            .is_some_and(|value| value == "true")
    {
        *matching_mmap = true;
    }
}

fn require_metadata(payload: &FilePayload, key: &'static str, failures: &mut Vec<String>) {
    if !payload.metadata.contains_key(key) {
        failures.push(format!("{} missing {key}", payload.operation));
    }
}

fn path_read_fault(payload: &FilePayload) -> bool {
    payload
        .metadata
        .get("path_read_fault")
        .is_some_and(|value| value == "true")
}

fn require_workload_file_io(
    events: &[DomainEvent],
    actions: &[SemanticAction],
    expected_file_path: &Path,
    failures: &mut Vec<String>,
) {
    let expected_path = expected_file_path.display().to_string();
    for (operation, action_kind, bytes_key) in [
        ("read", "file.read", attrs::file::BYTES_READ),
        ("write", "file.write", attrs::file::BYTES_WRITTEN),
    ] {
        if has_file_event(events, operation, &expected_path)
            || has_file_action(actions, action_kind, &expected_path, bytes_key)
        {
            continue;
        }
        failures.push(format!(
            "workload file {operation} missing retained event or semantic action for {expected_path}"
        ));
    }
}

fn has_file_event(events: &[DomainEvent], operation: &str, expected_path: &str) -> bool {
    events.iter().any(|event| {
        let EventPayload::File(payload) = &event.payload else {
            return false;
        };
        payload.operation == operation
            && payload.path.as_deref() == Some(expected_path)
            && payload
                .metadata
                .get("size")
                .and_then(|value| value.parse::<u64>().ok())
                .is_some_and(|size| size > 0)
    })
}

fn has_file_action(
    actions: &[SemanticAction],
    action_kind: &str,
    expected_path: &str,
    bytes_key: &str,
) -> bool {
    actions.iter().any(|action| {
        action.kind.as_str() == action_kind
            && action
                .attributes
                .get(attrs::file::PATH)
                .is_some_and(|path| path == expected_path)
            && positive_u64_attribute(action, bytes_key)
    })
}

fn positive_u64_attribute(action: &SemanticAction, key: &str) -> bool {
    action
        .attributes
        .get(key)
        .and_then(|value| value.parse::<u64>().ok())
        .is_some_and(|value| value > 0)
}
