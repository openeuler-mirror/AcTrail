//! SQLite assertions for live eBPF verification.

#[path = "database/file.rs"]
mod file;
#[path = "database/stdio.rs"]
mod stdio;

use std::collections::HashSet;
use std::path::Path;

use model_core::event::{DomainEvent, EventPayload};
use model_core::ids::TraceId;
use sqlite_storage::SqliteStorage;
use store_read_contract::events::EventReadStore;
use store_read_contract::payloads::{PayloadReadStore, PayloadSegmentQuery};

use crate::report::LiveVerificationReport;

pub(super) fn verify_database(
    storage_path: &Path,
    trace_id: TraceId,
    expected_provider: &str,
    expected_stdin: &str,
    expected_stdout: &str,
    expected_stderr: &str,
    expected_mmap: Option<(u64, u64)>,
    expect_resource_metrics: bool,
    expect_system_metrics: bool,
) -> Result<LiveVerificationReport, String> {
    let storage = SqliteStorage::open(storage_path).map_err(|error| error.to_string())?;
    let events = storage
        .list_events(trace_id)
        .map_err(|error| format!("query events failed: {}: {}", error.stage, error.message))?;
    let payload_segments = storage
        .list_payload_segments(
            trace_id,
            PayloadSegmentQuery {
                segment_id: None,
                direction: None,
                limit: None,
                include_bytes: true,
            },
        )
        .map_err(|error| format!("query payloads failed: {}: {}", error.stage, error.message))?;
    let observed_process = observed_process_operations(&events);
    let observed_file = file::observed(&events);
    let observed_net = observed_net_operations(&events);
    let observed_ipc = observed_ipc_operations(&events);
    let observed_resource = observed_resource_scopes(&events);
    let observed_providers = observed_provider_labels(&events);
    let observed_stdio_payloads = stdio::observed(&payload_segments);

    require_operations(
        "process",
        &observed_process,
        ["fork", "exec", "exit"].into_iter(),
    )?;
    require_operations(
        "net",
        &observed_net,
        ["bind", "listen", "connect", "accept", "send", "recv"].into_iter(),
    )?;
    let mut required_file_operations = vec![
        "open", "write", "read", "mkdir", "rmdir", "rename", "unlink", "truncate",
    ];
    if expected_mmap.is_some() {
        required_file_operations.push("mmap_shared");
    }
    require_operations("file", &observed_file, required_file_operations.into_iter())?;
    require_operations(
        "ipc",
        &observed_ipc,
        [
            "pipe:write",
            "pipe:read",
            "fifo:write",
            "fifo:read",
            "unix_socket:write",
            "unix_socket:read",
        ]
        .into_iter(),
    )?;
    require_process_payloads(&events)?;
    reject_default_signal_payloads(&events)?;
    file::require(&events, expected_mmap)?;
    require_net_payloads(&events)?;
    require_ipc_payloads(&events)?;
    require_provider_payloads(&events, expected_provider)?;
    require_resource_payloads(&events, expect_resource_metrics, expect_system_metrics)?;
    stdio::require(
        &payload_segments,
        expected_stdin,
        expected_stdout,
        expected_stderr,
    )?;

    Ok(LiveVerificationReport {
        trace_id,
        process_events: sorted(observed_process),
        file_events: sorted(observed_file),
        net_events: sorted(observed_net),
        ipc_events: sorted(observed_ipc),
        resource_events: sorted(observed_resource),
        provider_events: sorted(observed_providers),
        stdio_payloads: sorted(observed_stdio_payloads),
    })
}

fn observed_process_operations(events: &[DomainEvent]) -> HashSet<String> {
    events
        .iter()
        .filter_map(|event| match &event.payload {
            EventPayload::Process(payload) => Some(payload.operation.clone()),
            _ => None,
        })
        .collect()
}

fn observed_net_operations(events: &[DomainEvent]) -> HashSet<String> {
    events
        .iter()
        .filter_map(|event| match &event.payload {
            EventPayload::Net(payload) => payload.metadata.get("operation").cloned(),
            _ => None,
        })
        .collect()
}

fn observed_ipc_operations(events: &[DomainEvent]) -> HashSet<String> {
    events
        .iter()
        .filter_map(|event| match &event.payload {
            EventPayload::Ipc(payload) => payload
                .metadata
                .get("operation")
                .map(|operation| format!("{}:{operation}", payload.channel)),
            _ => None,
        })
        .collect()
}

fn observed_provider_labels(events: &[DomainEvent]) -> HashSet<String> {
    events
        .iter()
        .filter_map(|event| match &event.payload {
            EventPayload::Label(payload) => Some(payload.provider.clone()),
            _ => None,
        })
        .collect()
}

fn observed_resource_scopes(events: &[DomainEvent]) -> HashSet<String> {
    events
        .iter()
        .filter_map(|event| match &event.payload {
            EventPayload::Resource(payload) => Some(payload.scope.clone()),
            _ => None,
        })
        .collect()
}

fn require_operations<'a>(
    label: &str,
    observed: &HashSet<String>,
    required: impl Iterator<Item = &'a str>,
) -> Result<(), String> {
    let missing = required
        .filter(|operation| !observed.contains(*operation))
        .collect::<Vec<_>>();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "missing {label} operations: {}; observed: {}",
            missing.join(","),
            sorted(observed.clone()).join(",")
        ))
    }
}

fn require_net_payloads(events: &[DomainEvent]) -> Result<(), String> {
    let mut failures = Vec::new();
    for event in events {
        let EventPayload::Net(payload) = &event.payload else {
            continue;
        };
        let operation = payload
            .metadata
            .get("operation")
            .map(String::as_str)
            .unwrap_or("unknown");
        if payload.local.is_none() && payload.remote.is_none() {
            failures.push(format!("{operation} missing local and remote endpoint"));
        }
        if payload.result.is_none() {
            failures.push(format!("{operation} missing syscall result"));
        }
        if matches!(operation, "send" | "recv") && payload.size.is_none() {
            failures.push(format!("{operation} missing transferred size"));
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("; "))
    }
}

fn require_ipc_payloads(events: &[DomainEvent]) -> Result<(), String> {
    let mut failures = Vec::new();
    for event in events {
        let EventPayload::Ipc(payload) = &event.payload else {
            continue;
        };
        let operation = payload
            .metadata
            .get("operation")
            .map(String::as_str)
            .unwrap_or("unknown");
        if payload.peer.is_none() {
            failures.push(format!("{operation} missing peer fd target"));
        }
        if payload.size.is_none() {
            failures.push(format!("{operation} missing transferred size"));
        }
        if !payload.metadata.contains_key("fd") {
            failures.push(format!("{operation} missing fd"));
        }
        if !payload.metadata.contains_key("result") {
            failures.push(format!("{operation} missing syscall result"));
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("; "))
    }
}

fn require_process_payloads(events: &[DomainEvent]) -> Result<(), String> {
    let exec_with_executable = events.iter().any(|event| {
        matches!(
            &event.payload,
            EventPayload::Process(payload)
                if payload.operation == "exec" && payload.executable.is_some()
        )
    });
    let exec_with_context = events.iter().any(|event| {
        matches!(
            &event.payload,
            EventPayload::Process(payload)
                if payload.operation == "exec"
                    && payload.metadata.contains_key("command_line")
                    && payload.metadata.contains_key("cwd")
                    && payload.metadata.contains_key("uid_effective")
                    && payload.metadata.contains_key("gid_effective")
        )
    });
    let exec_with_resource_context = events.iter().any(|event| {
        matches!(
            &event.payload,
            EventPayload::Process(payload)
                if payload.operation == "exec"
                    && payload.metadata.contains_key("vm_size_kb")
                    && payload.metadata.contains_key("vm_rss_kb")
                    && payload.metadata.contains_key("threads")
                    && payload.metadata.contains_key("process_group_id")
                    && payload.metadata.contains_key("session_id")
        )
    });
    let exit_without_code = events.iter().any(|event| {
        matches!(
            &event.payload,
            EventPayload::Process(payload)
                if payload.operation == "exit" && !payload.metadata.contains_key("exit_code")
        )
    });

    if exec_with_executable && exec_with_context && exec_with_resource_context && !exit_without_code
    {
        Ok(())
    } else {
        Err(format!(
            "missing lifecycle payload details: exec_executable={}, exec_context={}, exec_resource_context={}, all_exit_codes={}",
            exec_with_executable, exec_with_context, exec_with_resource_context, !exit_without_code
        ))
    }
}

fn reject_default_signal_payloads(events: &[DomainEvent]) -> Result<(), String> {
    let Some(payload) = events.iter().find_map(|event| match &event.payload {
        EventPayload::Process(payload) if payload.operation == "signal" => Some(payload),
        _ => None,
    }) else {
        return Ok(());
    };
    Err(format!(
        "default proc-lifecycle unexpectedly persisted process signal event syscall={} signal={}",
        payload.metadata.get("syscall").cloned().unwrap_or_default(),
        payload.metadata.get("signal").cloned().unwrap_or_default()
    ))
}

fn require_provider_payloads(
    events: &[DomainEvent],
    expected_provider: &str,
) -> Result<(), String> {
    let mut matching_labels = events.iter().filter_map(|event| match &event.payload {
        EventPayload::Label(payload) if payload.provider == expected_provider => Some(payload),
        _ => None,
    });
    let Some(label) = matching_labels.next() else {
        return Err(format!("missing provider label {expected_provider}"));
    };
    if label.confidence_millis.is_none() {
        return Err(format!(
            "provider label {expected_provider} missing confidence_millis"
        ));
    }
    if label.evidence.is_empty() {
        return Err(format!(
            "provider label {expected_provider} missing evidence"
        ));
    }
    Ok(())
}

fn require_resource_payloads(
    events: &[DomainEvent],
    expect_resource_metrics: bool,
    expect_system_metrics: bool,
) -> Result<(), String> {
    if !expect_resource_metrics {
        return Ok(());
    }
    let Some(payload) = events.iter().find_map(|event| match &event.payload {
        EventPayload::Resource(payload) => Some(payload),
        _ => None,
    }) else {
        return Err("missing resource metrics event".to_string());
    };
    if payload.subject.is_empty() {
        return Err("resource metrics event missing subject".to_string());
    }
    if payload.rss_kb.is_none() {
        return Err("resource metrics event missing rss_kb".to_string());
    }
    if payload.virtual_memory_kb.is_none() {
        return Err("resource metrics event missing virtual_memory_kb".to_string());
    }
    if !payload.metadata.contains_key("sampled_processes") {
        return Err("resource metrics event missing sampled_processes".to_string());
    }
    if !payload.metadata.contains_key("include_children") {
        return Err("resource metrics event missing include_children".to_string());
    }
    if expect_system_metrics {
        for key in [
            "host_mem_total_kb",
            "host_mem_available_kb",
            "host_loadavg_1m",
        ] {
            if !payload.metadata.contains_key(key) {
                return Err(format!("resource metrics event missing {key}"));
            }
        }
    }
    Ok(())
}

fn sorted(values: HashSet<String>) -> Vec<String> {
    let mut values = values.into_iter().collect::<Vec<_>>();
    values.sort();
    values
}
