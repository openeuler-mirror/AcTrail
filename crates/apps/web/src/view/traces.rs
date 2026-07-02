//! Trace JSON rendering for the web API.

use model_core::trace::TraceRecord;

use crate::json;

pub(super) fn trace_record_json(trace: &TraceRecord) -> String {
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
        "container_id",
        &json::optional_string(trace.root_container_id.as_deref()),
    );
    output.push(',');
    json::field(
        &mut output,
        "state",
        &json::string(trace.lifecycle_state.as_display_str()),
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
        "created_at_unix_nanos",
        &json::time_nanos(trace.timings.created_at),
    );
    output.push(',');
    json::field(
        &mut output,
        "started_at",
        &json::optional_time(trace.timings.started_at),
    );
    output.push(',');
    json::field(
        &mut output,
        "started_at_unix_nanos",
        &json::optional_time_nanos(trace.timings.started_at),
    );
    output.push(',');
    json::field(
        &mut output,
        "completed_at",
        &json::optional_time(trace.timings.completed_at),
    );
    output.push(',');
    json::field(
        &mut output,
        "completed_at_unix_nanos",
        &json::optional_time_nanos(trace.timings.completed_at),
    );
    output.push(',');
    json::field(
        &mut output,
        "exited_at",
        &json::optional_time(trace.timings.exited_at),
    );
    output.push(',');
    json::field(
        &mut output,
        "exited_at_unix_nanos",
        &json::optional_time_nanos(trace.timings.exited_at),
    );
    output.push(',');
    json::field(
        &mut output,
        "failed_at",
        &json::optional_time(trace.timings.failed_at),
    );
    output.push(',');
    json::field(
        &mut output,
        "failed_at_unix_nanos",
        &json::optional_time_nanos(trace.timings.failed_at),
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
