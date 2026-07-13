//! Derived timeline and process-tree JSON views.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use model_core::event::{DomainEvent, EventPayload};
use model_core::payload::PayloadSegment;
use model_core::process::{ProcessIdentity, ProcessMembership};

use crate::json;
use crate::view::traces::ProcessDisplayIndex;

struct TimelineRow {
    observed_at: SystemTime,
    sort_id: u64,
    json: String,
}

pub(super) fn process_tree_json(
    memberships: &[ProcessMembership],
    process_display: &ProcessDisplayIndex,
) -> String {
    let parents = memberships
        .iter()
        .map(|membership| (membership.identity, membership.inherited_from))
        .collect::<BTreeMap<_, _>>();
    let mut children = BTreeMap::<ProcessIdentity, usize>::new();
    for parent in parents.values().flatten() {
        *children.entry(*parent).or_default() += 1;
    }
    let mut rows = memberships.iter().collect::<Vec<_>>();
    rows.sort_by_key(|membership| {
        (
            process_depth(membership.identity, &parents),
            membership.inherited_from.unwrap_or(membership.identity),
            membership.identity,
        )
    });
    format!(
        "[{}]",
        rows.iter()
            .map(|membership| {
                process_tree_node_json(membership, &parents, &children, process_display)
            })
            .collect::<Vec<_>>()
            .join(",")
    )
}

pub(super) fn timeline_json(events: &[DomainEvent], payloads: &[PayloadSegment]) -> String {
    let mut rows = Vec::new();
    for event in events {
        rows.push(TimelineRow {
            observed_at: event.envelope.observed_at,
            sort_id: event.envelope.event_id.get(),
            json: event_timeline_json(event),
        });
    }
    for segment in payloads {
        rows.push(TimelineRow {
            observed_at: segment.observed_at,
            sort_id: segment.segment_id.get(),
            json: payload_timeline_json(segment),
        });
    }
    rows.sort_by(|left, right| {
        sort_key(left.observed_at)
            .cmp(&sort_key(right.observed_at))
            .then(left.sort_id.cmp(&right.sort_id))
    });
    format!(
        "[{}]",
        rows.into_iter()
            .map(|row| row.json)
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn process_tree_node_json(
    membership: &ProcessMembership,
    parents: &BTreeMap<ProcessIdentity, Option<ProcessIdentity>>,
    children: &BTreeMap<ProcessIdentity, usize>,
    process_display: &ProcessDisplayIndex,
) -> String {
    let mut output = String::from("{");
    json::field(
        &mut output,
        "process_id",
        &json::number(membership.identity.get()),
    );
    output.push(',');
    json::field(
        &mut output,
        "pid",
        &json::optional_number(process_display.host_pid(membership.identity)),
    );
    output.push(',');
    json::field(
        &mut output,
        "parent_process_id",
        &json::optional_number(membership.inherited_from.map(|parent| parent.get())),
    );
    output.push(',');
    json::field(
        &mut output,
        "parent_pid",
        &json::optional_number(
            membership
                .inherited_from
                .and_then(|parent| process_display.host_pid(parent)),
        ),
    );
    output.push(',');
    json::field(
        &mut output,
        "depth",
        &json::number(process_depth(membership.identity, parents)),
    );
    output.push(',');
    json::field(
        &mut output,
        "children",
        &json::number(
            children
                .get(&membership.identity)
                .copied()
                .unwrap_or_default(),
        ),
    );
    output.push(',');
    json::field(
        &mut output,
        "state",
        &json::string(&format!("{:?}", membership.state)),
    );
    output.push('}');
    output
}

fn event_timeline_json(event: &DomainEvent) -> String {
    timeline_row_json(
        "event",
        event.envelope.event_id.get(),
        event.envelope.observed_at,
        event.envelope.process,
        event_lane(&event.payload),
        &event_title(&event.payload),
        &event_summary(&event.payload),
    )
}

fn payload_timeline_json(segment: &PayloadSegment) -> String {
    timeline_row_json(
        "payload",
        segment.segment_id.get(),
        segment.observed_at,
        segment.process,
        "payload",
        &format!("{:?} {:?}", segment.source_boundary, segment.direction),
        segment.protocol_hint.as_deref().unwrap_or(&segment.symbol),
    )
}

fn timeline_row_json(
    kind: &str,
    id: u64,
    observed_at: SystemTime,
    process: ProcessIdentity,
    lane: &str,
    title: &str,
    summary: &str,
) -> String {
    let mut output = String::from("{");
    json::field(&mut output, "kind", &json::string(kind));
    output.push(',');
    json::field(&mut output, "id", &json::number(id));
    output.push(',');
    json::field(&mut output, "observed_at", &json::time(observed_at));
    output.push(',');
    json::field(
        &mut output,
        "observed_at_unix_nanos",
        &json::time_nanos(observed_at),
    );
    output.push(',');
    json::field(&mut output, "process_id", &json::number(process.get()));
    output.push(',');
    json::field(&mut output, "lane", &json::string(lane));
    output.push(',');
    json::field(&mut output, "title", &json::string(title));
    output.push(',');
    json::field(&mut output, "summary", &json::string(summary));
    output.push('}');
    output
}

fn process_depth(
    process: ProcessIdentity,
    parents: &BTreeMap<ProcessIdentity, Option<ProcessIdentity>>,
) -> usize {
    let mut depth = 0;
    let mut current = process;
    let mut visited = BTreeSet::new();
    while let Some(Some(parent)) = parents.get(&current) {
        if !visited.insert(current) {
            break;
        }
        depth += 1;
        current = *parent;
    }
    depth
}

fn sort_key(value: SystemTime) -> Option<Duration> {
    value.duration_since(UNIX_EPOCH).ok()
}

fn event_lane(payload: &EventPayload) -> &'static str {
    match payload {
        EventPayload::Process(_) => "process",
        EventPayload::Net(_) => "network",
        EventPayload::File(_) => "file",
        EventPayload::Ipc(_) => "ipc",
        EventPayload::Stdio(_) => "stdio",
        EventPayload::Application(_) => "application",
        EventPayload::Resource(_) => "resource",
        EventPayload::Control(_) => "control",
        EventPayload::Loss(_) => "loss",
        EventPayload::Label(_) => "label",
        EventPayload::Enforcement(_) => "enforcement",
    }
}

fn event_title(payload: &EventPayload) -> String {
    match payload {
        EventPayload::Process(payload) => payload.operation.clone(),
        EventPayload::Net(payload) => payload
            .metadata
            .get("operation")
            .cloned()
            .unwrap_or_default(),
        EventPayload::File(payload) => payload.operation.clone(),
        EventPayload::Ipc(payload) => payload.channel.clone(),
        EventPayload::Stdio(payload) => payload.stream.clone(),
        EventPayload::Application(payload) => payload.operation.clone(),
        EventPayload::Resource(payload) => payload.scope.clone(),
        EventPayload::Control(payload) => payload.action.clone(),
        EventPayload::Loss(payload) => payload.reason.clone(),
        EventPayload::Label(payload) => payload.provider.clone(),
        EventPayload::Enforcement(payload) => payload.operation.clone(),
    }
}

fn event_summary(payload: &EventPayload) -> String {
    match payload {
        EventPayload::Process(payload) => payload
            .executable
            .clone()
            .or_else(|| payload.metadata.get("target_pid").cloned())
            .unwrap_or_default(),
        EventPayload::Net(payload) => format!(
            "{} -> {}",
            payload.local.clone().unwrap_or_default(),
            payload.remote.clone().unwrap_or_default()
        ),
        EventPayload::File(payload) => payload.path.clone().unwrap_or_default(),
        EventPayload::Ipc(payload) => payload.peer.clone().unwrap_or_default(),
        EventPayload::Stdio(payload) => format!("{} bytes", payload.data.len()),
        EventPayload::Application(payload) => payload.summary.clone(),
        EventPayload::Resource(payload) => resource_summary(payload),
        EventPayload::Control(payload) => payload.detail.clone(),
        EventPayload::Loss(payload) => format!("fatal={}", payload.fatal),
        EventPayload::Label(payload) => payload
            .confidence_millis
            .map(|value| format!("confidence_millis={value}"))
            .unwrap_or_default(),
        EventPayload::Enforcement(payload) => format!(
            "{} {}",
            payload.decision,
            payload.path.clone().unwrap_or_default()
        ),
    }
}

fn resource_summary(payload: &model_core::event::ResourcePayload) -> String {
    format!(
        "{} cpu={} rss_kb={}",
        payload.subject,
        payload
            .cpu_percent_millis
            .map(format_percent_millis)
            .unwrap_or_default(),
        payload
            .rss_kb
            .map(|value| value.to_string())
            .unwrap_or_default()
    )
}

fn format_percent_millis(value: u64) -> String {
    format!("{}.{:03}", value / 1000, value % 1000)
}
