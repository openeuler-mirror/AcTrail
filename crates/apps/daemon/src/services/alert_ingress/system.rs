use std::collections::BTreeMap;
use std::sync::Arc;

use alert_contract::{AlertDefinition, AlertDraft, AlertSeverity};
use control_contract::reply::ControlError;
use plugin_system::PluginRuntimeError;
use serde::Serialize;
use storage_core::StorageBackend;

use super::protocol::AlertAdmission;

pub(super) const DAEMON_ENFORCEMENT_INSTANCE_ID: &str = "actraild.enforcement";
const FILE_DEFINITION_KEY: &str = "file-access-boundary-violation";
const FILE_PAYLOAD_SCHEMA_ID: &str = "actrail.file-access-boundary-violation.payload.v1";
const COMMAND_DEFINITION_KEY: &str = "command-execution-boundary-violation";
const COMMAND_PAYLOAD_SCHEMA_ID: &str = "actrail.command-execution-boundary-violation.payload.v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum FileAccessDenySource {
    FastPathDeny,
    GrayPluginDeny,
    GrayPluginCacheDeny,
}

#[derive(Debug, Eq, PartialEq, Serialize)]
pub(crate) struct FileAccessBoundaryAlert {
    operation: String,
    path: String,
    matched_rule_path: String,
    rule_id: String,
    policy_owner_instance_id: String,
    process_id: u64,
    decision_source: FileAccessDenySource,
    #[serde(skip_serializing_if = "Option::is_none")]
    decider_plugin_instance_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    plugin_reason: Option<String>,
}

impl FileAccessBoundaryAlert {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        operation: String,
        path: String,
        matched_rule_path: String,
        rule_id: String,
        policy_owner_instance_id: String,
        process_id: u64,
        decision_source: FileAccessDenySource,
        decider_plugin_instance_id: Option<String>,
        plugin_reason: Option<String>,
    ) -> Self {
        Self {
            operation,
            path,
            matched_rule_path,
            rule_id,
            policy_owner_instance_id,
            process_id,
            decision_source,
            decider_plugin_instance_id,
            plugin_reason,
        }
    }

    pub(super) fn into_draft(self) -> Result<AlertDraft, PluginRuntimeError> {
        let payload_json = serde_json::to_string(&self).map_err(|error| {
            PluginRuntimeError::new(
                "daemon_alert_payload",
                format!("serialize file access boundary alert: {error}"),
            )
        })?;
        Ok(AlertDraft {
            definition_key: FILE_DEFINITION_KEY.to_string(),
            payload_json,
            deduplication_key: None,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum CommandExecutionDenySource {
    FastPathDeny,
    GrayPluginDeny,
    GrayPluginCacheDeny,
}

#[derive(Debug, Eq, PartialEq, Serialize)]
pub(crate) struct CommandExecutionBoundaryAlert {
    operation: String,
    executable: String,
    rule_id: String,
    policy_owner_instance_id: String,
    process_id: u64,
    decision_source: CommandExecutionDenySource,
    #[serde(skip_serializing_if = "Option::is_none")]
    decider_plugin_instance_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    plugin_reason: Option<String>,
}

impl CommandExecutionBoundaryAlert {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        operation: String,
        executable: String,
        rule_id: String,
        policy_owner_instance_id: String,
        process_id: u64,
        decision_source: CommandExecutionDenySource,
        decider_plugin_instance_id: Option<String>,
        plugin_reason: Option<String>,
    ) -> Self {
        Self {
            operation,
            executable,
            rule_id,
            policy_owner_instance_id,
            process_id,
            decision_source,
            decider_plugin_instance_id,
            plugin_reason,
        }
    }

    pub(super) fn into_draft(self) -> Result<AlertDraft, PluginRuntimeError> {
        let payload_json = serde_json::to_string(&self).map_err(|error| {
            PluginRuntimeError::new(
                "daemon_alert_payload",
                format!("serialize command execution boundary alert: {error}"),
            )
        })?;
        Ok(AlertDraft {
            definition_key: COMMAND_DEFINITION_KEY.to_string(),
            payload_json,
            deduplication_key: None,
        })
    }
}

pub(super) fn register(
    storage: &mut dyn StorageBackend,
) -> Result<Arc<AlertAdmission>, ControlError> {
    let file_definition = AlertDefinition {
        producer_plugin_id: DAEMON_ENFORCEMENT_INSTANCE_ID.to_string(),
        definition_key: FILE_DEFINITION_KEY.to_string(),
        kind: "file.access.boundary-violation".to_string(),
        title: "Out-of-bound file access denied".to_string(),
        severity: AlertSeverity::High,
        payload_schema_id: FILE_PAYLOAD_SCHEMA_ID.to_string(),
    };
    storage
        .register_alert_definition(&file_definition)
        .map_err(|error| ControlError::new(error.stage, error.message))?;
    let command_definition = AlertDefinition {
        producer_plugin_id: DAEMON_ENFORCEMENT_INSTANCE_ID.to_string(),
        definition_key: COMMAND_DEFINITION_KEY.to_string(),
        kind: "command.execution.boundary-violation".to_string(),
        title: "Out-of-bound command execution denied".to_string(),
        severity: AlertSeverity::High,
        payload_schema_id: COMMAND_PAYLOAD_SCHEMA_ID.to_string(),
    };
    storage
        .register_alert_definition(&command_definition)
        .map_err(|error| ControlError::new(error.stage, error.message))?;
    Ok(Arc::new(AlertAdmission::new(
        DAEMON_ENFORCEMENT_INSTANCE_ID.to_string(),
        DAEMON_ENFORCEMENT_INSTANCE_ID.to_string(),
        BTreeMap::new(),
        BTreeMap::from([
            (FILE_DEFINITION_KEY.to_string(), file_definition),
            (COMMAND_DEFINITION_KEY.to_string(), command_definition),
        ]),
    )))
}
