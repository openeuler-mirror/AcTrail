//! SQLite-backed API view rendering.

#[path = "view/events.rs"]
mod events;
#[path = "view/payloads.rs"]
mod payloads;
#[path = "view/topology.rs"]
mod topology;

use std::path::Path;

use model_core::ids::TraceId;
use model_core::payload::PayloadSegmentId;
use model_core::trace::TraceRecord;
use sqlite_storage::SqliteStorage;
use store_read_contract::filters::TraceFilter;
use store_read_contract::payloads::{PayloadReadStore, PayloadSegmentQuery};
use store_read_contract::traces::TraceReadStore;
use store_snapshot_contract::lease::SnapshotLeaseStore;
use store_snapshot_contract::view::SnapshotStore;

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
    json::field(
        &mut output,
        "diagnostics",
        &format!("[{}]", diagnostics.join(",")),
    );
    output.push('}');
    Ok(output)
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
