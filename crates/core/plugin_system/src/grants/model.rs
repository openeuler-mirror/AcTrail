//! Capability grant sets and policy publication authorization.
mod host_grant;
mod policy_grants;
mod wire;
pub use host_grant::PluginHostGrant;
pub use policy_grants::{
    CommandPolicyRulesApplyGrant, FilePolicyRulesApplyGrant, NetworkPolicyRulesApplyGrant,
};
use std::collections::BTreeSet;

use model_core::payload::PayloadSourceBoundary;

use crate::{
    CommandPolicyDecision, FilePolicyDecision, NetworkPolicyDecision,
    NetworkPolicyEndpointGrantScope, NetworkPolicyOperation, NetworkPolicyProtocol,
    PluginCapability,
};

use super::parser::ScopedApplyGrantParser;
use super::validation::GrantValidator;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PluginHostGrants {
    http_request: bool,
    payload_read_all: bool,
    payload_read_sources: BTreeSet<String>,
    context_query: bool,
    trace_analysis_read: bool,
    trace_activity_read: bool,
    trace_file_state_read: bool,
    alert_write: bool,
    file_access_current_match_get: bool,
    file_access_current_context_query: bool,
    file_policy_rules_read: bool,
    file_policy_rules_match_dry_run: bool,
    file_policy_rules_validate: bool,
    file_policy_rules_apply: Vec<FilePolicyRulesApplyGrant>,
    command_execution_current_context_query: bool,
    command_policy_rules_read: bool,
    command_policy_rules_match_dry_run: bool,
    command_policy_rules_validate: bool,
    command_policy_rules_apply: Vec<CommandPolicyRulesApplyGrant>,
    network_action_current_context_query: bool,
    network_policy_rules_read: bool,
    network_policy_rules_match_dry_run: bool,
    network_policy_rules_validate: bool,
    network_policy_rules_apply: Vec<NetworkPolicyRulesApplyGrant>,
    env_read: BTreeSet<String>,
}

impl PluginHostGrants {
    pub fn parse(values: &[String]) -> Result<Self, String> {
        let mut grants = Self::default();
        let mut seen = BTreeSet::<&str>::new();
        for value in values {
            if !seen.insert(value.as_str()) {
                return Err(format!("duplicate plugin host grant {value}"));
            }
            match PluginHostGrant::parse(value)? {
                PluginHostGrant::HttpRequest => grants.http_request = true,
                PluginHostGrant::PayloadRead => grants.allow_payload_read(),
                PluginHostGrant::PayloadReadSource { source } => {
                    grants.allow_payload_read_source(source)?
                }
                PluginHostGrant::ContextQuery => grants.allow_context_query(),
                PluginHostGrant::TraceAnalysisRead => grants.allow_trace_analysis_read(),
                PluginHostGrant::TraceActivityRead => grants.allow_trace_activity_read(),
                PluginHostGrant::TraceFileStateRead => grants.allow_trace_file_state_read(),
                PluginHostGrant::AlertWrite => grants.allow_alert_write(),
                PluginHostGrant::FileAccessCurrentMatchGet => {
                    grants.allow_file_access_current_match_get()
                }
                PluginHostGrant::FileAccessCurrentContextQuery => {
                    grants.allow_file_access_current_context_query()
                }
                PluginHostGrant::FilePolicyRulesRead => grants.allow_file_policy_rules_read(),
                PluginHostGrant::FilePolicyRulesMatchDryRun => {
                    grants.allow_file_policy_rules_match_dry_run()
                }
                PluginHostGrant::FilePolicyRulesValidate => {
                    grants.allow_file_policy_rules_validate()
                }
                PluginHostGrant::FilePolicyRulesApply { decision, path } => {
                    grants.allow_file_policy_rules_apply(decision, path)?
                }
                PluginHostGrant::CommandExecutionCurrentContextQuery => {
                    grants.allow_command_execution_current_context_query()
                }
                PluginHostGrant::CommandPolicyRulesRead => grants.allow_command_policy_rules_read(),
                PluginHostGrant::CommandPolicyRulesMatchDryRun => {
                    grants.allow_command_policy_rules_match_dry_run()
                }
                PluginHostGrant::CommandPolicyRulesValidate => {
                    grants.allow_command_policy_rules_validate()
                }
                PluginHostGrant::CommandPolicyRulesApply { decision, path } => {
                    grants.allow_command_policy_rules_apply(decision, path)?
                }
                PluginHostGrant::NetworkActionCurrentContextQuery => {
                    grants.allow_network_action_current_context_query()
                }
                PluginHostGrant::NetworkPolicyRulesRead => grants.allow_network_policy_rules_read(),
                PluginHostGrant::NetworkPolicyRulesMatchDryRun => {
                    grants.allow_network_policy_rules_match_dry_run()
                }
                PluginHostGrant::NetworkPolicyRulesValidate => {
                    grants.allow_network_policy_rules_validate()
                }
                PluginHostGrant::NetworkPolicyRulesApply {
                    decision,
                    operation,
                    protocol,
                    endpoint,
                } => grants
                    .allow_network_policy_rules_apply(decision, operation, protocol, endpoint)?,
                PluginHostGrant::EnvRead { name } => grants.allow_env_read(name)?,
            }
        }
        if grants.payload_read_all && !grants.payload_read_sources.is_empty() {
            return Err(
                "cannot combine payload-read with payload-read:source scoped grants".to_string(),
            );
        }
        Ok(grants)
    }

    pub fn allow_payload_read(&mut self) {
        self.payload_read_all = true;
    }

    pub fn allow_payload_read_source(&mut self, source: impl Into<String>) -> Result<(), String> {
        let source = source.into();
        GrantValidator::payload_source(&source)?;
        self.payload_read_sources.insert(source);
        Ok(())
    }

    pub fn can_read_payload(&self) -> bool {
        self.payload_read_all || !self.payload_read_sources.is_empty()
    }

    pub fn can_read_payload_source(&self, source_boundary: PayloadSourceBoundary) -> bool {
        self.payload_read_all
            || self
                .payload_read_sources
                .contains(GrantValidator::payload_source_name(source_boundary))
    }

    pub fn allow_env_read(&mut self, name: impl Into<String>) -> Result<(), String> {
        let name = name.into();
        GrantValidator::env_name(&name)?;
        self.env_read.insert(name);
        Ok(())
    }

    pub fn allow_context_query(&mut self) {
        self.context_query = true;
    }

    pub fn can_request_http(&self) -> bool {
        self.http_request
    }

    pub fn can_query_context(&self) -> bool {
        self.context_query
    }

    pub fn allow_trace_analysis_read(&mut self) {
        self.trace_analysis_read = true;
    }

    pub fn can_read_trace_analysis(&self) -> bool {
        self.trace_analysis_read
    }

    pub fn allow_trace_activity_read(&mut self) {
        self.trace_activity_read = true;
    }

    pub fn can_read_trace_activity(&self) -> bool {
        self.trace_activity_read
    }

    pub fn allow_trace_file_state_read(&mut self) {
        self.trace_file_state_read = true;
    }

    pub fn can_read_trace_file_state(&self) -> bool {
        self.trace_file_state_read
    }

    pub fn allow_alert_write(&mut self) {
        self.alert_write = true;
    }

    pub fn can_write_alerts(&self) -> bool {
        self.alert_write
    }

    pub fn allow_file_access_current_match_get(&mut self) {
        self.file_access_current_match_get = true;
    }

    pub fn can_get_current_file_access_match(&self) -> bool {
        self.file_access_current_match_get
    }

    pub fn allow_file_access_current_context_query(&mut self) {
        self.file_access_current_context_query = true;
    }

    pub fn can_query_current_file_access_context(&self) -> bool {
        self.file_access_current_context_query
    }

    pub fn allow_file_policy_rules_read(&mut self) {
        self.file_policy_rules_read = true;
    }

    pub fn can_read_file_policy_rules(&self) -> bool {
        self.file_policy_rules_read
    }

    pub fn allow_file_policy_rules_match_dry_run(&mut self) {
        self.file_policy_rules_match_dry_run = true;
    }

    pub fn can_match_dry_run_file_policy_rules(&self) -> bool {
        self.file_policy_rules_match_dry_run
    }

    pub fn allow_file_policy_rules_validate(&mut self) {
        self.file_policy_rules_validate = true;
    }

    pub fn can_validate_file_policy_rules(&self) -> bool {
        self.file_policy_rules_validate
    }

    pub fn allow_file_policy_rules_apply(
        &mut self,
        decision: FilePolicyDecision,
        path_scope: impl Into<String>,
    ) -> Result<(), String> {
        GrantValidator::file_decision(decision)?;
        let path_scope = path_scope.into();
        GrantValidator::file_path_scope(&path_scope)?;
        self.file_policy_rules_apply
            .push(FilePolicyRulesApplyGrant {
                decision,
                path_scope,
            });
        Ok(())
    }

    pub fn file_policy_rules_apply_grants(&self) -> &[FilePolicyRulesApplyGrant] {
        &self.file_policy_rules_apply
    }

    pub fn can_apply_file_policy_rules(&self) -> bool {
        !self.file_policy_rules_apply.is_empty()
    }

    pub fn allow_command_execution_current_context_query(&mut self) {
        self.command_execution_current_context_query = true;
    }

    pub fn can_query_current_command_execution_context(&self) -> bool {
        self.command_execution_current_context_query
    }

    pub fn allow_command_policy_rules_read(&mut self) {
        self.command_policy_rules_read = true;
    }

    pub fn can_read_command_policy_rules(&self) -> bool {
        self.command_policy_rules_read
    }

    pub fn allow_command_policy_rules_match_dry_run(&mut self) {
        self.command_policy_rules_match_dry_run = true;
    }

    pub fn can_match_dry_run_command_policy_rules(&self) -> bool {
        self.command_policy_rules_match_dry_run
    }

    pub fn allow_command_policy_rules_validate(&mut self) {
        self.command_policy_rules_validate = true;
    }

    pub fn can_validate_command_policy_rules(&self) -> bool {
        self.command_policy_rules_validate
    }

    pub fn allow_command_policy_rules_apply(
        &mut self,
        decision: CommandPolicyDecision,
        path_scope: impl Into<String>,
    ) -> Result<(), String> {
        GrantValidator::command_decision(decision)?;
        let path_scope = path_scope.into();
        GrantValidator::command_path_scope(&path_scope)?;
        self.command_policy_rules_apply
            .push(CommandPolicyRulesApplyGrant {
                decision,
                path_scope,
            });
        Ok(())
    }

    pub fn command_policy_rules_apply_grants(&self) -> &[CommandPolicyRulesApplyGrant] {
        &self.command_policy_rules_apply
    }

    pub fn can_apply_command_policy_rules(&self) -> bool {
        !self.command_policy_rules_apply.is_empty()
    }

    pub fn allow_network_action_current_context_query(&mut self) {
        self.network_action_current_context_query = true;
    }

    pub fn can_query_current_network_action_context(&self) -> bool {
        self.network_action_current_context_query
    }

    pub fn allow_network_policy_rules_read(&mut self) {
        self.network_policy_rules_read = true;
    }

    pub fn can_read_network_policy_rules(&self) -> bool {
        self.network_policy_rules_read
    }

    pub fn allow_network_policy_rules_match_dry_run(&mut self) {
        self.network_policy_rules_match_dry_run = true;
    }

    pub fn can_match_dry_run_network_policy_rules(&self) -> bool {
        self.network_policy_rules_match_dry_run
    }

    pub fn allow_network_policy_rules_validate(&mut self) {
        self.network_policy_rules_validate = true;
    }

    pub fn can_validate_network_policy_rules(&self) -> bool {
        self.network_policy_rules_validate
    }

    pub fn allow_network_policy_rules_apply(
        &mut self,
        decision: NetworkPolicyDecision,
        operation: NetworkPolicyOperation,
        protocol: NetworkPolicyProtocol,
        endpoint_scope: impl Into<String>,
    ) -> Result<(), String> {
        GrantValidator::network_decision(decision)?;
        let endpoint_scope = GrantValidator::network_endpoint_scope(&endpoint_scope.into())?;
        self.network_policy_rules_apply
            .push(NetworkPolicyRulesApplyGrant {
                decision,
                operation,
                protocol,
                endpoint_scope,
            });
        Ok(())
    }

    pub fn network_policy_rules_apply_grants(&self) -> &[NetworkPolicyRulesApplyGrant] {
        &self.network_policy_rules_apply
    }

    pub fn can_apply_network_policy_rules(&self) -> bool {
        !self.network_policy_rules_apply.is_empty()
    }

    pub fn can_read_env(&self, name: &str) -> bool {
        self.env_read.contains(name)
    }

    pub fn env_read_names(&self) -> impl Iterator<Item = &str> {
        self.env_read.iter().map(String::as_str)
    }

    pub fn is_empty(&self) -> bool {
        !self.http_request
            && !self.payload_read_all
            && self.payload_read_sources.is_empty()
            && !self.context_query
            && !self.trace_analysis_read
            && !self.trace_activity_read
            && !self.trace_file_state_read
            && !self.alert_write
            && !self.file_access_current_match_get
            && !self.file_access_current_context_query
            && !self.file_policy_rules_read
            && !self.file_policy_rules_match_dry_run
            && !self.file_policy_rules_validate
            && self.file_policy_rules_apply.is_empty()
            && !self.command_execution_current_context_query
            && !self.command_policy_rules_read
            && !self.command_policy_rules_match_dry_run
            && !self.command_policy_rules_validate
            && self.command_policy_rules_apply.is_empty()
            && !self.network_action_current_context_query
            && !self.network_policy_rules_read
            && !self.network_policy_rules_match_dry_run
            && !self.network_policy_rules_validate
            && self.network_policy_rules_apply.is_empty()
            && self.env_read.is_empty()
    }
}
