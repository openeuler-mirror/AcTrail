//! Wire encoding of granted host capabilities.
use super::{PluginHostGrant, PluginHostGrants};

impl PluginHostGrants {
    pub fn to_wire_values(&self) -> Vec<String> {
        let mut values = Vec::new();
        if self.http_request {
            values.push(PluginHostGrant::HttpRequest.to_wire());
        }
        if self.payload_read_all {
            values.push(PluginHostGrant::PayloadRead.to_wire());
        }
        values.extend(self.payload_read_sources.iter().map(|source| {
            PluginHostGrant::PayloadReadSource {
                source: source.clone(),
            }
            .to_wire()
        }));
        if self.context_query {
            values.push(PluginHostGrant::ContextQuery.to_wire());
        }
        if self.trace_analysis_read {
            values.push(PluginHostGrant::TraceAnalysisRead.to_wire());
        }
        if self.trace_activity_read {
            values.push(PluginHostGrant::TraceActivityRead.to_wire());
        }
        if self.trace_file_state_read {
            values.push(PluginHostGrant::TraceFileStateRead.to_wire());
        }
        if self.alert_write {
            values.push(PluginHostGrant::AlertWrite.to_wire());
        }
        if self.file_access_current_match_get {
            values.push(PluginHostGrant::FileAccessCurrentMatchGet.to_wire());
        }
        if self.file_access_current_context_query {
            values.push(PluginHostGrant::FileAccessCurrentContextQuery.to_wire());
        }
        if self.file_policy_rules_read {
            values.push(PluginHostGrant::FilePolicyRulesRead.to_wire());
        }
        if self.file_policy_rules_match_dry_run {
            values.push(PluginHostGrant::FilePolicyRulesMatchDryRun.to_wire());
        }
        if self.file_policy_rules_validate {
            values.push(PluginHostGrant::FilePolicyRulesValidate.to_wire());
        }
        values.extend(self.file_policy_rules_apply.iter().map(|grant| {
            PluginHostGrant::FilePolicyRulesApply {
                decision: grant.decision,
                path: grant.path_scope.clone(),
            }
            .to_wire()
        }));
        if self.command_execution_current_context_query {
            values.push(PluginHostGrant::CommandExecutionCurrentContextQuery.to_wire());
        }
        if self.command_policy_rules_read {
            values.push(PluginHostGrant::CommandPolicyRulesRead.to_wire());
        }
        if self.command_policy_rules_match_dry_run {
            values.push(PluginHostGrant::CommandPolicyRulesMatchDryRun.to_wire());
        }
        if self.command_policy_rules_validate {
            values.push(PluginHostGrant::CommandPolicyRulesValidate.to_wire());
        }
        values.extend(self.command_policy_rules_apply.iter().map(|grant| {
            PluginHostGrant::CommandPolicyRulesApply {
                decision: grant.decision,
                path: grant.path_scope.clone(),
            }
            .to_wire()
        }));
        if self.network_action_current_context_query {
            values.push(PluginHostGrant::NetworkActionCurrentContextQuery.to_wire());
        }
        if self.network_policy_rules_read {
            values.push(PluginHostGrant::NetworkPolicyRulesRead.to_wire());
        }
        if self.network_policy_rules_match_dry_run {
            values.push(PluginHostGrant::NetworkPolicyRulesMatchDryRun.to_wire());
        }
        if self.network_policy_rules_validate {
            values.push(PluginHostGrant::NetworkPolicyRulesValidate.to_wire());
        }
        values.extend(self.network_policy_rules_apply.iter().map(|grant| {
            PluginHostGrant::NetworkPolicyRulesApply {
                decision: grant.decision,
                operation: grant.operation,
                protocol: grant.protocol,
                endpoint: grant.endpoint_scope.to_string(),
            }
            .to_wire()
        }));
        values.extend(
            self.env_read
                .iter()
                .map(|name| PluginHostGrant::EnvRead { name: name.clone() }.to_wire()),
        );
        values
    }
}
