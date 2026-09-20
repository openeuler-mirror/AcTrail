use agent_host::ExecutionKey;
use alert_contract::{AlertDefinition, AlertDraft, AlertSeverity};
use model_core::ids::TraceId;
use serde_json::{Value, json};
use std::time::{Duration, SystemTime};

pub(super) const PRODUCER_ID: &str = "actrail.agent-idle";

pub struct IdleAlert {
    pub trace_id: TraceId,
    pub draft: AlertDraft,
}

impl IdleAlert {
    pub(super) fn new(
        key: &ExecutionKey,
        started: SystemTime,
        observed: SystemTime,
        threshold: Duration,
        recovered: bool,
    ) -> Self {
        let start = started
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
            .to_string();
        let observed = observed
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
            .to_string();
        let kind = if recovered {
            "hang_recovered"
        } else {
            "hang_detected"
        };
        let mut payload = json!({
            "episode_key": json!([key.trace_id.get().to_string(), key.session_id, key.task_id, start]).to_string(),
            "session_id": key.session_id, "task_id": key.task_id, "kind": kind,
            "started_at_unix_nanos": start, "observed_at_unix_nanos": observed,
            "threshold_nanos": threshold.as_nanos().to_string()
        });
        if recovered {
            payload["ended_at_unix_nanos"] = payload["observed_at_unix_nanos"].clone();
        }
        Self {
            trace_id: key.trace_id,
            draft: AlertDraft {
                definition_key: kind.to_owned(),
                payload_json: payload.to_string(),
                deduplication_key: None,
            },
        }
    }

    pub(super) fn definitions() -> Vec<(AlertDefinition, Value)> {
        [("hang_detected", "Agent execution stalled", AlertSeverity::Medium),
         ("hang_recovered", "Agent execution resumed", AlertSeverity::Informational)]
            .into_iter().map(|(kind, title, severity)| {
                let schema_id = format!("actrail://agent-idle/{kind}/v1");
                let mut required = vec!["episode_key", "session_id", "task_id", "kind", "started_at_unix_nanos", "observed_at_unix_nanos", "threshold_nanos"];
                let mut properties = json!({
                    "episode_key": {"type":"string", "minLength":1}, "session_id": {"type":"string", "minLength":1},
                    "task_id": {"type":"string", "minLength":1}, "kind": {"const":kind},
                    "started_at_unix_nanos": {"type":"string", "pattern":"^[0-9]+$"},
                    "observed_at_unix_nanos": {"type":"string", "pattern":"^[0-9]+$"},
                    "threshold_nanos": {"type":"string", "pattern":"^[0-9]+$"}
                });
                if kind == "hang_recovered" {
                    required.push("ended_at_unix_nanos");
                    properties["ended_at_unix_nanos"] = json!({"type":"string", "pattern":"^[0-9]+$"});
                }
                let schema = json!({"$schema":"https://json-schema.org/draft/2020-12/schema", "$id":schema_id,
                    "type":"object", "additionalProperties":false, "required":required, "properties":properties});
                (AlertDefinition { producer_plugin_id: PRODUCER_ID.to_owned(), definition_key: kind.to_owned(),
                    kind: format!("agent.{kind}"), title: title.to_owned(), severity, payload_schema_id: schema_id }, schema)
            }).collect()
    }
}
