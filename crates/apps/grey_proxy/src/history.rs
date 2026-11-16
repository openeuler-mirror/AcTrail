//! Read a bounded set of recent decision and execution facts for one process.
use crate::config::Config;
use model_core::{ids::TraceId, process::ProcessIdentity};
use semantic_action::{SemanticAction, SemanticActionKind, SemanticActionStatus};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use storage_core::{RecentActionsQuery, StorageBackend, StorageOpenMode};
use storage_factory::{StorageConfig, open_storage_backend};

pub(crate) struct HistoryReader {
    storage: Box<dyn StorageBackend>,
    window: Duration,
    limit: usize,
}
impl HistoryReader {
    pub fn open(config: &Config) -> Result<Self, String> {
        let storage = open_storage_backend(
            &StorageConfig::sqlite_path(&config.storage_path),
            StorageOpenMode::ReadOnly,
        )
        .map_err(|e| format!("{}: {}", e.stage, e.message))?;
        Ok(Self {
            storage,
            window: Duration::from_millis(config.grey_proxy.window_ms),
            limit: config.grey_proxy.max_actions,
        })
    }
    pub fn read(&self, request: &Value) -> Value {
        match self.query(request) {
            Ok(context) => context,
            Err(reason) => json!({"error": reason, "actions": [], "truncated": false}),
        }
    }
    fn query(&self, request: &Value) -> Result<Value, String> {
        let process = request["logical_process_id"]
            .as_u64()
            .filter(|id| *id > 0)
            .ok_or("logical process identity unavailable")?;
        let trace = request["trace_id"]
            .as_str()
            .ok_or("trace identity unavailable")?;
        let trace = TraceId::new(
            trace
                .strip_prefix("trace-")
                .unwrap_or(trace)
                .parse::<u64>()
                .map_err(|e| e.to_string())?,
        );
        let end = SystemTime::now();
        let start = end
            .checked_sub(self.window)
            .ok_or("history window exceeds clock range")?;
        let mut ids = self
            .storage
            .recent_action_ids(&RecentActionsQuery {
                trace_id: trace,
                process: ProcessIdentity::new(process),
                kinds: vec![
                    SemanticActionKind::EnforcementDecision,
                    SemanticActionKind::ProcessExec,
                ],
                start,
                end,
                limit: self.limit.checked_add(1).ok_or("history limit overflow")?,
            })
            .map_err(|e| format!("{}: {}", e.stage, e.message))?;
        let truncated = ids.len() > self.limit;
        ids.truncate(self.limit);
        // Hydrate only the selected IDs. Other callers keep their lightweight timeline reads.
        let selected: Vec<_> = ids.iter().map(String::as_str).collect();
        let details = self
            .storage
            .semantic_actions_by_ids_lite(trace, &selected)
            .map_err(|e| format!("{}: {}", e.stage, e.message))?;
        let by_id: HashMap<_, _> = details
            .iter()
            .map(|action| (action.action_id.as_str(), action))
            .collect();
        let actions: Vec<_> = selected
            .iter()
            .filter_map(|id| by_id.get(id))
            .map(|action| Self::fact(action))
            .collect();
        Ok(json!({"actions": actions, "truncated": truncated}))
    }
    fn fact(action: &SemanticAction) -> Value {
        let attrs = &action.attributes;
        let mut fact = json!({"time": action.start_time.duration_since(UNIX_EPOCH).unwrap_or_default().as_millis()});
        if action.kind == SemanticActionKind::EnforcementDecision {
            fact["operation"] = json!(attrs.get("enforcement.operation"));
            fact["decision"] = json!(attrs.get("enforcement.decision"));
            fact["target"] = json!(
                attrs
                    .get("file.path")
                    .or_else(|| attrs.get("remote_endpoint"))
                    .or_else(|| attrs.get("local_endpoint"))
            );
            if let Some(protocol) = attrs.get("protocol") {
                fact["protocol"] = json!(protocol);
            }
        } else {
            fact["operation"] = "exec".into();
            if action.status != SemanticActionStatus::Success {
                fact["result"] = action.status.as_str().into();
            }
            if let Some(command) = attrs.get("command_line") {
                // Preserve the collected command text; it is not a lossless argv array.
                fact["command"] = json!(command);
                if attrs
                    .get("args_truncated")
                    .is_some_and(|v| v == "true" || v == "1")
                {
                    fact["parameters_truncated"] = true.into();
                }
            } else {
                fact["target"] = json!(attrs.get("process.executable").unwrap_or(&action.title));
                fact["parameters_unavailable"] = true.into();
            }
        }
        fact
    }
}
