//! WIT 0.4 component interface names and record-field constants.

pub const CONTROL_DECIDER_EXPORT: &str = "actrail:plugin/control-decider@0.4.0";
pub const CONTROL_DECIDE_EXPORT: &str = "decide";
pub const MANAGEMENT_COMMAND_EXPORT: &str = "actrail:plugin/management-command@0.4.0";
pub const MANAGEMENT_HANDLE_COMMAND_EXPORT: &str = "handle-command";
pub const MANAGEMENT_HANDLE_COMMAND_FLAT_EXPORT: &str =
    "actrail:plugin/management-command@0.4.0#handle-command";
pub const RUNTIME_CONFIG_EXPORT: &str = "actrail:plugin/runtime-config@0.4.0";
pub const RUNTIME_CONFIG_GET_EXPORT: &str = "get";
pub const RUNTIME_CONFIG_VALIDATE_EXPORT: &str = "validate";
pub const RUNTIME_CONFIG_SUBMIT_EXPORT: &str = "submit";
pub const HOST_IMPORT: &str = "actrail:plugin/host@0.4.0";
pub const NETWORK_CONTROL_HOST_IMPORT: &str = "actrail:plugin/network-control-host@0.4.0";

pub mod host_import {
    pub const READ_CONFIG: &str = "read-config";
    pub const QUERY_CONTEXT: &str = "query-context";
    pub const FILE_ACCESS_CURRENT_MATCH_GET: &str = "file-access-current-match-get";
    pub const FILE_POLICY_RULES_VERSION_GET: &str = "file-policy-rules-version-get";
    pub const FILE_POLICY_RULES_LIST: &str = "file-policy-rules-list";
    pub const FILE_POLICY_RULES_MATCH_DRY_RUN: &str = "file-policy-rules-match-dry-run";
    pub const FILE_POLICY_RULES_VALIDATE: &str = "file-policy-rules-validate";
    pub const FILE_POLICY_RULES_APPLY: &str = "file-policy-rules-apply";
    pub const COMMAND_EXECUTION_CURRENT_CONTEXT_QUERY: &str =
        "command-execution-current-context-query";
    pub const COMMAND_POLICY_RULES_VERSION_GET: &str = "command-policy-rules-version-get";
    pub const COMMAND_POLICY_RULES_LIST: &str = "command-policy-rules-list";
    pub const COMMAND_POLICY_RULES_MATCH_DRY_RUN: &str = "command-policy-rules-match-dry-run";
    pub const COMMAND_POLICY_RULES_VALIDATE: &str = "command-policy-rules-validate";
    pub const COMMAND_POLICY_RULES_APPLY: &str = "command-policy-rules-apply";
}

pub mod network_host_import {
    pub const NETWORK_ACTION_CURRENT_CONTEXT_QUERY: &str = "network-action-current-context-query";
    pub const NETWORK_POLICY_RULES_VERSION_GET: &str = "network-policy-rules-version-get";
    pub const NETWORK_POLICY_RULES_LIST: &str = "network-policy-rules-list";
    pub const NETWORK_POLICY_RULES_MATCH_DRY_RUN: &str = "network-policy-rules-match-dry-run";
    pub const NETWORK_POLICY_RULES_VALIDATE: &str = "network-policy-rules-validate";
    pub const NETWORK_POLICY_RULES_APPLY: &str = "network-policy-rules-apply";
}

pub mod grant {
    pub const CONTEXT_QUERY: &str = "context-query";
    pub const FILE_ACCESS_CURRENT_MATCH_GET: &str = "file-access.current-match-get";
    pub const FILE_POLICY_RULES_READ: &str = "file-policy.rules.read";
    pub const FILE_POLICY_RULES_MATCH_DRY_RUN: &str = "file-policy.rules.match-dry-run";
    pub const FILE_POLICY_RULES_VALIDATE: &str = "file-policy.rules.validate";
    pub const FILE_POLICY_RULES_APPLY_PREFIX: &str = "file-policy.rules.apply:";
    pub const COMMAND_EXECUTION_CURRENT_CONTEXT_QUERY: &str =
        "command-execution.current-context-query";
    pub const COMMAND_POLICY_RULES_READ: &str = "command-policy.rules.read";
    pub const COMMAND_POLICY_RULES_MATCH_DRY_RUN: &str = "command-policy.rules.match-dry-run";
    pub const COMMAND_POLICY_RULES_VALIDATE: &str = "command-policy.rules.validate";
    pub const COMMAND_POLICY_RULES_APPLY_PREFIX: &str = "command-policy.rules.apply:";
    pub const NETWORK_ACTION_CURRENT_CONTEXT_QUERY: &str = "network-action.current-context-query";
    pub const NETWORK_POLICY_RULES_READ: &str = "network-policy.rules.read";
    pub const NETWORK_POLICY_RULES_MATCH_DRY_RUN: &str = "network-policy.rules.match-dry-run";
    pub const NETWORK_POLICY_RULES_VALIDATE: &str = "network-policy.rules.validate";
    pub const NETWORK_POLICY_RULES_APPLY_PREFIX: &str = "network-policy.rules.apply:";
}

pub mod decision_request {
    pub const DECISION_ID: &str = "decision-id";
    pub const TRACE_ID: &str = "trace-id";
    pub const TASK_ID: &str = "task-id";
    pub const SUBJECT: &str = "subject";
    pub const ACTOR_PROCESS_IDENTITY: &str = "actor-process-identity";
    pub const OPERATION: &str = "operation";
    pub const TARGET_SUMMARY: &str = "target-summary";
    pub const CONTEXT_REF: &str = "context-ref";
}

pub mod plugin_command_request {
    pub const ARGV: &str = "argv";
}

pub mod plugin_command_result {
    pub const EXIT_CODE: &str = "exit-code";
    pub const STDOUT: &str = "stdout";
    pub const STDERR: &str = "stderr";
}

pub mod actor_process {
    pub const PID: &str = "pid";
    pub const TASK_ID: &str = "task-id";
    pub const GENERATION: &str = "generation";
    pub const NAMESPACE: &str = "namespace";
}

pub mod decision_summary {
    pub const SUBJECT: &str = "subject";
    pub const OPERATION: &str = "operation";
    pub const TARGET_SUMMARY: &str = "target-summary";
    pub const DECISION_ID: &str = "decision-id";
    pub const TRACE_ID: &str = "trace-id";
    pub const ACTOR_PROCESS_IDENTITY: &str = "actor-process-identity";
}

pub mod file_policy_view {
    pub const RULE_ID: &str = "rule-id";
    pub const DECISION: &str = "decision";
    pub const OPERATION: &str = "operation";
    pub const PATH: &str = "path";
    pub const PLUGIN_INSTANCE: &str = "plugin-instance";
    pub const TIMEOUT_MS: &str = "timeout-ms";
    pub const CONCURRENCY_LIMIT: &str = "concurrency-limit";
    pub const FALLBACK: &str = "fallback";
}

pub mod file_policy_list_filter {
    pub const DECISION: &str = "decision";
    pub const PATH_PREFIX: &str = "path-prefix";
    pub const OPERATION: &str = "operation";
}

pub mod file_policy_rule_view {
    pub const RULE_ID: &str = "rule-id";
    pub const OWNER_INSTANCE_ID: &str = "owner-instance-id";
    pub const DECISION: &str = "decision";
    pub const OPERATION: &str = "operation";
    pub const PATH: &str = "path";
    pub const GRAY_TARGET: &str = "gray-target";
    pub const PRIORITY: &str = "priority";
    pub const ENABLED: &str = "enabled";
    pub const UPDATED_SEQUENCE: &str = "updated-sequence";
}

pub mod file_policy_list_result {
    pub const RULES: &str = "rules";
    pub const NEXT_CURSOR: &str = "next-cursor";
    pub const SOURCE_REVISION: &str = "source-revision";
}

pub mod file_policy_match_dry_run {
    pub const PATH: &str = "path";
    pub const OPERATION: &str = "operation";
    pub const MATCHED: &str = "matched";
    pub const DECISION: &str = "decision";
    pub const RULE_ID: &str = "rule-id";
    pub const CANONICAL_PATH: &str = "canonical-path";
    pub const SOURCE_REVISION: &str = "source-revision";
}

pub mod command_policy {
    pub const RULE_ID: &str = "rule-id";
    pub const OWNER_INSTANCE_ID: &str = "owner-instance-id";
    pub const DECISION: &str = "decision";
    pub const EXECUTABLE: &str = "executable";
    pub const ARGS: &str = "args";
    pub const EXECUTABLE_PREFIX: &str = "executable-prefix";
    pub const GRAY_TARGET: &str = "gray-target";
    pub const PRIORITY: &str = "priority";
    pub const RULE_REVISION: &str = "rule-revision";
    pub const UPDATED_SEQUENCE: &str = "updated-sequence";
    pub const RULES: &str = "rules";
    pub const NEXT_CURSOR: &str = "next-cursor";
    pub const SOURCE_REVISION: &str = "source-revision";
    pub const MATCHED: &str = "matched";
    pub const RESOLVED_EXECUTABLE: &str = "resolved-executable";
}

pub mod network_policy {
    pub const DECISION: &str = "decision";
    pub const REMOTE: &str = "remote";
}
