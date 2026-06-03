//! SQLite-backed API view rendering.

#[path = "view/actions.rs"]
mod actions;
#[path = "view/events.rs"]
mod events;
#[path = "view/payloads.rs"]
mod payloads;
#[path = "view/topology.rs"]
mod topology;

use std::collections::BTreeMap;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use model_core::ids::TraceId;
use model_core::payload::PayloadSegmentId;
use model_core::trace::TraceRecord;
use sqlite_storage::SqliteStorage;
use store_read_contract::filters::TraceFilter;
use store_read_contract::payloads::{PayloadReadStore, PayloadSegmentQuery};
use store_read_contract::traces::TraceReadStore;
use store_snapshot_contract::lease::SnapshotLeaseStore;
use store_snapshot_contract::view::{SnapshotStore, SnapshotView};

use crate::json;

pub fn traces_json(storage_path: &Path) -> Result<String, String> {
    let storage = open_storage(storage_path)?;
    let traces = storage
        .list_traces(&TraceFilter::default())
        .map_err(|error| format!("list traces failed: {}: {}", error.stage, error.message))?;
    let rows = traces.iter().map(trace_record_json).collect::<Vec<_>>();
    Ok(format!("{{\"traces\":[{}]}}", rows.join(",")))
}

pub fn trace_json(storage_path: &Path, trace_id: u64) -> Result<String, String> {
    let mut storage = open_storage(storage_path)?;
    let trace_id = TraceId::new(trace_id);
    let lease = storage.acquire_export_lease(trace_id).map_err(|error| {
        format!(
            "acquire snapshot lease failed: {}: {}",
            error.stage, error.message
        )
    })?;
    let snapshot = storage
        .read_snapshot(&lease)
        .map_err(|error| format!("read snapshot failed: {}: {}", error.stage, error.message))?;
    storage.release_export_lease(lease).map_err(|error| {
        format!(
            "release snapshot lease failed: {}: {}",
            error.stage, error.message
        )
    })?;
    let payloads = storage
        .list_payload_segments(trace_id, PayloadSegmentQuery::metadata_only(None))
        .map_err(|error| format!("list payloads failed: {}: {}", error.stage, error.message))?;
    let retained_payload_bytes = storage.retained_payload_bytes(trace_id).map_err(|error| {
        format!(
            "read retained payload bytes failed: {}: {}",
            error.stage, error.message
        )
    })?;
    let counts = events::event_counts(&snapshot.events);
    let events = snapshot
        .events
        .iter()
        .map(events::event_json)
        .collect::<Vec<_>>();
    let processes = snapshot
        .memberships
        .iter()
        .map(events::process_json)
        .collect::<Vec<_>>();
    let diagnostics = snapshot
        .diagnostics
        .iter()
        .map(events::diagnostic_json)
        .collect::<Vec<_>>();
    let payload_rows = payloads
        .iter()
        .map(payloads::payload_json_row)
        .collect::<Vec<_>>();
    let process_tree = topology::process_tree_json(&snapshot.memberships);
    let timeline = topology::timeline_json(&snapshot.events, &payloads);

    // Compute analysis data for Commands tab (process lifetimes with Unix timestamps)
    let analysis = compute_analysis(&snapshot);

    let mut output = String::from("{");
    json::field(&mut output, "trace", &trace_record_json(&snapshot.trace));
    output.push(',');
    json::field(
        &mut output,
        "counts",
        &events::counts_json(&counts, retained_payload_bytes),
    );
    output.push(',');
    json::field(&mut output, "events", &format!("[{}]", events.join(",")));
    output.push(',');
    json::field(
        &mut output,
        "processes",
        &format!("[{}]", processes.join(",")),
    );
    output.push(',');
    json::field(&mut output, "process_tree", &process_tree);
    output.push(',');
    json::field(
        &mut output,
        "payloads",
        &format!("[{}]", payload_rows.join(",")),
    );
    output.push(',');
    json::field(&mut output, "timeline", &timeline);
    output.push(',');
    json::field(&mut output, "diagnostics", &format!("[{}]", diagnostics.join(",")));
    output.push(',');
    json::field(&mut output, "analysis", &analysis);
    output.push('}');
    Ok(output)
}

pub fn action_tree_json(storage_path: &Path, trace_id: u64) -> Result<String, String> {
    let mut storage = open_storage(storage_path)?;
    actions::action_tree_json(&mut storage, TraceId::new(trace_id))
}

pub fn payload_json(storage_path: &Path, trace_id: u64, segment_id: u64) -> Result<String, String> {
    let storage = open_storage(storage_path)?;
    let mut segments = storage
        .list_payload_segments(
            TraceId::new(trace_id),
            PayloadSegmentQuery {
                segment_id: Some(PayloadSegmentId::new(segment_id)),
                direction: None,
                limit: None,
                include_bytes: true,
            },
        )
        .map_err(|error| format!("list payload failed: {}: {}", error.stage, error.message))?;
    let segment = segments
        .pop()
        .ok_or_else(|| format!("payload segment {segment_id} not found"))?;
    Ok(payloads::payload_json_with_bytes(&segment))
}

fn open_storage(storage_path: &Path) -> Result<SqliteStorage, String> {
    SqliteStorage::open_read_only(storage_path)
        .map_err(|error| format!("open storage read-only failed: {error}"))
}

fn trace_record_json(trace: &TraceRecord) -> String {
    let mut output = String::from("{");
    json::field(&mut output, "id", &json::number(trace.trace_id.get()));
    output.push(',');
    json::field(
        &mut output,
        "display_id",
        &json::string(&trace.trace_id.to_string()),
    );
    output.push(',');
    json::field(
        &mut output,
        "name",
        &json::string(trace.display_name.as_str()),
    );
    output.push(',');
    json::field(
        &mut output,
        "profile",
        &json::string(trace.profile_name.as_str()),
    );
    output.push(',');
    json::field(
        &mut output,
        "root_pid",
        &json::number(trace.root_process_identity.pid),
    );
    output.push(',');
    json::field(
        &mut output,
        "state",
        &json::string(&format!("{:?}", trace.lifecycle_state)),
    );
    output.push(',');
    json::field(
        &mut output,
        "health",
        &json::string(&format!("{:?}", trace.health)),
    );
    output.push(',');
    json::field(
        &mut output,
        "created_at",
        &json::time(trace.timings.created_at),
    );
    output.push(',');
    json::field(
        &mut output,
        "tags",
        &json::string_array(trace.tags.iter().cloned()),
    );
    output.push('}');
    output
}

/// Compute analysis data: process lifetimes with Unix timestamps and durations,
/// plus command invocations (exec events paired with exit events).
fn compute_analysis(snapshot: &SnapshotView) -> String {
    use model_core::event::{EventPayload, ProcessPayload};

    let mut pid_start: BTreeMap<u32, u64> = BTreeMap::new();
    let mut pid_end: BTreeMap<u32, u64> = BTreeMap::new();

    // Collect earliest and latest event timestamps per PID
    for event in &snapshot.events {
        let pid = event.envelope.process.pid;
        let observed_millis = event
            .envelope
            .observed_at
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        pid_start
            .entry(pid)
            .and_modify(|v| {
                if observed_millis < *v {
                    *v = observed_millis;
                }
            })
            .or_insert(observed_millis);
        pid_end
            .entry(pid)
            .and_modify(|v| {
                if observed_millis > *v {
                    *v = observed_millis;
                }
            })
            .or_insert(observed_millis);
    }

    // Build process lifetimes array
    let mut proc_rows = Vec::new();
    for membership in &snapshot.memberships {
        let pid = membership.identity.pid;
        let parent_pid = membership.inherited_from.as_ref().map(|p| p.pid);
        let state = format!("{:?}", membership.state);

        let start = pid_start.get(&pid).copied();
        let end = pid_end.get(&pid).copied();
        let duration_ms = match (start, end) {
            (Some(s), Some(e)) => Some(e.saturating_sub(s)),
            _ => None,
        };

        let mut row = String::from("{");
        json::field(&mut row, "pid", &json::number(pid));
        row.push(',');
        json::field(&mut row, "parent_pid", &json::optional_number(parent_pid));
        row.push(',');
        json::field(&mut row, "start_unix_millis", &json::optional_number(start));
        row.push(',');
        json::field(&mut row, "end_unix_millis", &json::optional_number(end));
        row.push(',');
        json::field(&mut row, "duration_ms", &json::optional_number(duration_ms));
        row.push(',');
        json::field(&mut row, "state", &json::string(&state));
        row.push('}');
        proc_rows.push(row);
    }

    // Build commands array: exec events paired with exit events
    let mut pid_exec: BTreeMap<u32, (&ProcessPayload, SystemTime)> = BTreeMap::new();
    let mut pid_exit: BTreeMap<u32, (SystemTime, Option<i32>)> = BTreeMap::new();
    for event in &snapshot.events {
        let pid = event.envelope.process.pid;
        if let EventPayload::Process(pp) = &event.payload {
            if pp.operation == "exec" {
                pid_exec.entry(pid).or_insert((pp, event.envelope.observed_at));
            } else if pp.operation == "exit" {
                pid_exit.entry(pid).or_insert((
                    event.envelope.observed_at,
                    pp.metadata.get("exit_code").and_then(|s| s.parse::<i32>().ok()),
                ));
            }
        }
    }

    let mut cmd_rows = Vec::new();
    for (pid, (pp, exec_time)) in &pid_exec {
        // Prefer command_line (full command with args), fallback to executable
        let filename = pp
            .metadata
            .get("command_line")
            .or_else(|| pp.executable.as_ref())
            .or_else(|| pp.metadata.get("exec_filename"))
            .cloned()
            .unwrap_or_else(|| pp.operation.clone());

        // Use metadata start_unix_millis if available, otherwise fallback to event time
        let start_millis = pp
            .metadata
            .get("start_unix_millis")
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or_else(|| {
                exec_time
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0)
            });

        let (end_millis, exit_code) = match pid_exit.get(pid) {
            Some((exit_time, code)) => {
                let end = exit_time
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(start_millis);
                (end, *code)
            }
            None => (start_millis, None),
        };

        let duration_ms = end_millis.saturating_sub(start_millis);

        // Get parent_pid from metadata.ppid or memberships
        let parent_pid = pp
            .metadata
            .get("ppid")
            .and_then(|s| s.parse::<u32>().ok())
            .or_else(|| {
                snapshot
                    .memberships
                    .iter()
                    .find(|m| m.identity.pid == *pid)
                    .and_then(|m| m.inherited_from.as_ref().map(|p| p.pid))
            });

        let mut row = String::from("{");
        json::field(&mut row, "pid", &json::number(*pid));
        row.push(',');
        json::field(&mut row, "parent_pid", &json::optional_number(parent_pid));
        row.push(',');
        json::field(&mut row, "command", &json::string(&filename));
        row.push(',');
        json::field(&mut row, "start_unix_millis", &json::number(start_millis));
        row.push(',');
        json::field(&mut row, "end_unix_millis", &json::number(end_millis));
        row.push(',');
        json::field(&mut row, "duration_ms", &json::number(duration_ms));
        row.push(',');
        json::field(&mut row, "exit_code", &json::optional_number(exit_code));
        row.push('}');
        cmd_rows.push(row);
    }

    // Compute summary
    let total_procs = snapshot.memberships.len();
    let total_cmds = cmd_rows.len();

    // Build the full analysis object
    let mut output = String::from("{");
    json::field(&mut output, "process_lifetimes", &format!("[{}]", proc_rows.join(",")));
    output.push(',');
    json::field(&mut output, "commands", &format!("[{}]", cmd_rows.join(",")));
    output.push(',');
    json::field(&mut output, "total_processes", &json::number(total_procs));
    output.push(',');
    json::field(&mut output, "total_commands", &json::number(total_cmds));
    output.push('}');
    output
}
