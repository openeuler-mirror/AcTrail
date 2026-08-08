use std::time::{SystemTime, UNIX_EPOCH};

use model_core::ids::TraceId;
use model_core::trace::TraceLifecycleState;
use plugin_system::{
    ObservationBatch, ObservationConsumeReport, PluginDroppedRecord, PluginRuntimeError,
    TraceAnalysisAction, TraceAnalysisFileChange,
};
use semantic_action::{
    FileObservationPath, SemanticAction, SemanticActionKind, attr_keys as attrs,
};
use wasmtime::component::Val;

use crate::engine::WasmStore;

pub(super) fn observation_batch_val(
    batch: &ObservationBatch<'_>,
    sequence: u64,
    lifecycle_transition: Option<TraceLifecycleState>,
) -> Val {
    Val::Record(vec![
        (
            "batch-id".to_string(),
            Val::String(format!("{}:{sequence}", batch.trace.trace_id)),
        ),
        (
            "trace-id".to_string(),
            Val::String(batch.trace.trace_id.to_string()),
        ),
        ("trace-sequence".to_string(), Val::U64(sequence)),
        (
            "lifecycle-transition".to_string(),
            Val::Option(lifecycle_transition.map(lifecycle_state_val).map(Box::new)),
        ),
        (
            "families".to_string(),
            Val::List(observation_families(batch, lifecycle_transition)),
        ),
        (
            "semantic-actions".to_string(),
            Val::List(
                batch
                    .semantic_actions
                    .iter()
                    .map(|action| semantic_action_val(action, batch.file_observation_paths))
                    .collect(),
            ),
        ),
        (
            "payload-refs".to_string(),
            Val::List(
                batch
                    .payload_segments
                    .iter()
                    .map(|segment| {
                        Val::Record(vec![
                            (
                                "id".to_string(),
                                Val::String(segment.segment_id.to_string()),
                            ),
                            (
                                "trace-id".to_string(),
                                Val::String(segment.trace_id.to_string()),
                            ),
                        ])
                    })
                    .collect(),
            ),
        ),
    ])
}

fn observation_families(
    batch: &ObservationBatch<'_>,
    lifecycle_transition: Option<TraceLifecycleState>,
) -> Vec<Val> {
    let mut families = Vec::new();
    if !batch.semantic_actions.is_empty() {
        families.push(Val::Enum("semantic-action".to_string()));
    }
    if !batch.semantic_links.is_empty() {
        families.push(Val::Enum("semantic-action-link".to_string()));
    }
    if !batch.payload_segments.is_empty() {
        families.push(Val::Enum("payload-metadata".to_string()));
    }
    if lifecycle_transition.is_some() {
        families.push(Val::Enum("trace-lifecycle".to_string()));
    }
    families
}

pub(super) fn semantic_action_val(action: &SemanticAction, paths: &[FileObservationPath]) -> Val {
    Val::Record(vec![
        (
            "action-id".to_string(),
            Val::String(action.action_id.clone()),
        ),
        (
            "trace-id".to_string(),
            Val::String(action.trace_id.to_string()),
        ),
        (
            "kind".to_string(),
            Val::String(action.kind.as_str().to_string()),
        ),
        (
            "status".to_string(),
            Val::String(action.status.as_str().to_string()),
        ),
        (
            "completeness".to_string(),
            Val::String(action.completeness.as_str().to_string()),
        ),
        (
            "file-change".to_string(),
            Val::Option(file_change_val(action, paths).map(Box::new)),
        ),
        (
            "attributes".to_string(),
            Val::List(observation_attributes(action)),
        ),
    ])
}

/// 导出语义动作属性时，统一注入 process.id，便于插件按进程精确关联
/// command.invocation 与 process.exit（不改 WIT record 布局，保持 ABI 兼容）。
fn observation_attributes(action: &SemanticAction) -> Vec<Val> {
    let mut pairs = Vec::with_capacity(action.attributes.len() + 1);
    for (key, value) in &action.attributes {
        pairs.push(Val::Record(vec![
            ("key".to_string(), Val::String(key.clone())),
            ("value".to_string(), Val::String(value.clone())),
        ]));
    }
    if !action.attributes.contains_key(attrs::process::ID) {
        pairs.push(Val::Record(vec![
            (
                "key".to_string(),
                Val::String(attrs::process::ID.to_string()),
            ),
            ("value".to_string(), Val::String(action.process.to_string())),
        ]));
    }
    pairs
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::time::SystemTime;

    use model_core::ids::TraceId;
    use semantic_action::{
        SemanticAction, SemanticActionCompleteness, SemanticActionKind, SemanticActionStatus,
    };
    use wasmtime::component::Val;

    use super::observation_attributes;

    fn command_action(attributes: BTreeMap<String, String>) -> SemanticAction {
        SemanticAction {
            action_id: "action-1".to_string(),
            trace_id: TraceId::new(7),
            kind: SemanticActionKind::CommandInvocation,
            title: "ls /missing".to_string(),
            start_time: SystemTime::UNIX_EPOCH,
            end_time: None,
            process: model_core::process::ProcessIdentity::new(42),
            status: SemanticActionStatus::Success,
            completeness: SemanticActionCompleteness::Complete,
            confidence_millis: None,
            attributes,
            evidence: Vec::new(),
        }
    }

    fn attribute_pairs(value: Val) -> Vec<(String, String)> {
        let Val::List(items) = value else {
            panic!("observation attributes must serialize to a list")
        };
        items
            .into_iter()
            .map(|item| {
                let Val::Record(fields) = item else {
                    panic!("attribute pair must serialize to a record")
                };
                let key = fields
                    .iter()
                    .find(|(name, _)| name == "key")
                    .expect("attribute pair has key");
                let value = fields
                    .iter()
                    .find(|(name, _)| name == "value")
                    .expect("attribute pair has value");
                let Val::String(key) = &key.1 else {
                    panic!("attribute key must be a string")
                };
                let Val::String(value) = &value.1 else {
                    panic!("attribute value must be a string")
                };
                (key.clone(), value.clone())
            })
            .collect()
    }

    #[test]
    fn observation_attributes_inject_process_id() {
        let value = Val::List(observation_attributes(&command_action(BTreeMap::new())));
        let pairs = attribute_pairs(value);
        let process_id = pairs
            .iter()
            .find(|(key, _)| key == "process.id")
            .map(|(_, value)| value.as_str());
        assert_eq!(process_id, Some("process-42"));
    }

    #[test]
    fn observation_attributes_do_not_duplicate_process_id() {
        let mut attributes = BTreeMap::new();
        attributes.insert("process.id".to_string(), "process-1".to_string());
        let value = Val::List(observation_attributes(&command_action(attributes)));
        let pairs = attribute_pairs(value);
        let matches = pairs
            .iter()
            .filter(|(key, _)| key == "process.id")
            .collect::<Vec<_>>();
        assert_eq!(matches.len(), 1, "process.id must not be duplicated");
        assert_eq!(matches[0].1, "process-1");
    }
}

pub(super) fn trace_analysis_action_val(action: &TraceAnalysisAction) -> Val {
    Val::Record(vec![
        (
            "action-id".to_string(),
            Val::String(action.action_id.clone()),
        ),
        ("trace-id".to_string(), Val::String(String::new())),
        ("kind".to_string(), Val::String(action.kind.clone())),
        (
            "status".to_string(),
            Val::String(action.status.as_str().to_string()),
        ),
        (
            "completeness".to_string(),
            Val::String(action.completeness.as_str().to_string()),
        ),
        (
            "file-change".to_string(),
            Val::Option(
                action
                    .file_change
                    .as_ref()
                    .map(trace_analysis_file_change_val)
                    .map(Box::new),
            ),
        ),
        ("attributes".to_string(), Val::List(Vec::new())),
    ])
}

fn trace_analysis_file_change_val(change: &TraceAnalysisFileChange) -> Val {
    Val::Record(vec![
        (
            "operation".to_string(),
            Val::String(change.operation.clone()),
        ),
        (
            "change-kind".to_string(),
            Val::Enum(change.change_kind.as_str().to_string()),
        ),
        ("successful".to_string(), Val::Bool(change.successful)),
        (
            "path".to_string(),
            Val::Option(change.path.clone().map(Val::String).map(Box::new)),
        ),
        (
            "path-state".to_string(),
            Val::Enum(
                if change.path_complete {
                    "complete"
                } else {
                    "unavailable"
                }
                .to_string(),
            ),
        ),
    ])
}

fn file_change_val(action: &SemanticAction, paths: &[FileObservationPath]) -> Option<Val> {
    if !matches!(
        action.kind,
        SemanticActionKind::FileModify | SemanticActionKind::FileWrite
    ) {
        return None;
    }
    let path = paths
        .iter()
        .filter(|path| path.action_id == action.action_id)
        .min_by_key(|path| path.path_order)
        .map(|path| path.path.clone())
        .or_else(|| {
            action
                .attributes
                .get(semantic_action::attr_keys::file::PATH)
                .cloned()
        });
    let operation = action
        .attributes
        .get(semantic_action::attr_keys::file::OPERATION)
        .cloned()
        .unwrap_or_else(|| match action.kind {
            SemanticActionKind::FileWrite => "write".to_string(),
            _ => "unknown".to_string(),
        });
    Some(Val::Record(vec![
        ("operation".to_string(), Val::String(operation)),
        (
            "change-kind".to_string(),
            Val::Enum(
                action
                    .file_change_kind()
                    .unwrap_or(match action.kind {
                        SemanticActionKind::FileWrite => semantic_action::FileChangeKind::Modified,
                        _ => semantic_action::FileChangeKind::Unknown,
                    })
                    .as_str()
                    .to_string(),
            ),
        ),
        (
            "successful".to_string(),
            Val::Bool(action.status == semantic_action::SemanticActionStatus::Success),
        ),
        (
            "path".to_string(),
            Val::Option(path.clone().map(Val::String).map(Box::new)),
        ),
        (
            "path-state".to_string(),
            Val::Enum(
                if path.is_some() {
                    "complete"
                } else {
                    "unavailable"
                }
                .to_string(),
            ),
        ),
    ]))
}

fn lifecycle_state_val(state: TraceLifecycleState) -> Val {
    let state = match state {
        TraceLifecycleState::Starting => "starting",
        TraceLifecycleState::Active => "active",
        TraceLifecycleState::Draining => "draining",
        TraceLifecycleState::Completed => "completed",
        TraceLifecycleState::Exited => "exited",
        TraceLifecycleState::Failed => "failed",
    };
    Val::Enum(state.to_string())
}

pub(super) fn parse_observation_report(
    instance_id: &str,
    queue_capacity: u32,
    trace_id: TraceId,
    value: Val,
) -> Result<ObservationConsumeReport, PluginRuntimeError> {
    let report = match value {
        Val::Result(Ok(Some(ok))) => *ok,
        Val::Result(Ok(None)) => {
            return Err(PluginRuntimeError::new(
                "wasm_runtime",
                "wasm component consume returned ok without observation-report",
            ));
        }
        Val::Result(Err(Some(error))) => {
            let message = match *error {
                Val::String(message) => message,
                other => format!("{other:?}"),
            };
            return Err(PluginRuntimeError::new(
                "wasm_runtime",
                format!("wasm component consume returned error: {message}"),
            ));
        }
        Val::Result(Err(None)) => {
            return Err(PluginRuntimeError::new(
                "wasm_runtime",
                "wasm component consume returned error without message",
            ));
        }
        other => {
            return Err(PluginRuntimeError::new(
                "wasm_runtime",
                format!("wasm component consume returned invalid result {other:?}"),
            ));
        }
    };
    let Val::Record(fields) = report else {
        return Err(PluginRuntimeError::new(
            "wasm_runtime",
            format!("wasm component consume returned invalid report {report:?}"),
        ));
    };
    let _observed_records = report_field_u64(&fields, "observed-records")?;
    let dropped_records = report_field_u64(&fields, "dropped-records")?;
    if dropped_records == 0 {
        return Ok(ObservationConsumeReport::empty());
    }
    Ok(ObservationConsumeReport {
        dropped_records: vec![PluginDroppedRecord {
            trace_id: Some(trace_id),
            plugin_instance: instance_id.to_string(),
            reason: "wasm_component_reported_drop".to_string(),
            queue_capacity: Some(queue_capacity),
            dropped_records,
        }],
        reevaluate_at: None,
    })
}

fn report_field_u64(fields: &[(String, Val)], name: &str) -> Result<u64, PluginRuntimeError> {
    match fields.iter().find(|(field, _)| field == name) {
        Some((_, Val::U64(value))) => Ok(*value),
        Some((_, other)) => Err(PluginRuntimeError::new(
            "wasm_runtime",
            format!("wasm component report field {name} has invalid value {other:?}"),
        )),
        None => Err(PluginRuntimeError::new(
            "wasm_runtime",
            format!("wasm component report missing field {name}"),
        )),
    }
}

pub(super) fn component_call_error(
    store: &mut WasmStore,
    operation: &str,
    error: impl std::fmt::Display,
) -> PluginRuntimeError {
    if store.get_fuel().map(|fuel| fuel == 0).unwrap_or(false) {
        PluginRuntimeError::new(
            "wasm_runtime",
            format!("wasm fuel exhausted during component {operation}: {error}"),
        )
    } else {
        PluginRuntimeError::new(
            "wasm_runtime",
            format!("wasm component {operation} trapped: {error}"),
        )
    }
}

pub(super) fn system_time_millis(time: SystemTime) -> Result<u64, PluginRuntimeError> {
    let duration = time.duration_since(UNIX_EPOCH).map_err(|error| {
        PluginRuntimeError::new("wasm_runtime", format!("time precedes epoch: {error}"))
    })?;
    u64::try_from(duration.as_millis()).map_err(|error| {
        PluginRuntimeError::new(
            "wasm_runtime",
            format!("time milliseconds overflow: {error}"),
        )
    })
}
