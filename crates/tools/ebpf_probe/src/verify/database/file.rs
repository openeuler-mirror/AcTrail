//! File event assertions for live eBPF verification.

use std::collections::HashSet;

use model_core::event::{DomainEvent, EventPayload, FilePayload};

pub(super) fn observed(events: &[DomainEvent]) -> HashSet<String> {
    events
        .iter()
        .filter_map(|event| match &event.payload {
            EventPayload::File(payload) => Some(payload.operation.clone()),
            _ => None,
        })
        .collect()
}

pub(super) fn require(
    events: &[DomainEvent],
    expected_mmap: Option<(u64, u64)>,
) -> Result<(), String> {
    let mut failures = Vec::new();
    let mut matching_mmap = false;
    for event in events {
        let EventPayload::File(payload) = &event.payload else {
            continue;
        };
        if payload.path.is_none() {
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
            "open" | "mkdir" | "rmdir" | "unlink" => {}
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
