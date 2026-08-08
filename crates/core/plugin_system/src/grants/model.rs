use std::collections::BTreeSet;

use model_core::payload::PayloadSourceBoundary;

use crate::{CommandPolicyDecision, FilePolicyDecision, PluginCapability};

use super::parser::ScopedApplyGrantParser;
use super::validation::GrantValidator;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PluginHostGrants {
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
    env_read: BTreeSet<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FilePolicyRulesApplyGrant {
    pub decision: FilePolicyDecision,
    pub path_scope: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandPolicyRulesApplyGrant {
    pub decision: CommandPolicyDecision,
    pub path_scope: String,
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

    pub fn can_read_env(&self, name: &str) -> bool {
        self.env_read.contains(name)
    }

    pub fn env_read_names(&self) -> impl Iterator<Item = &str> {
        self.env_read.iter().map(String::as_str)
    }

    pub fn to_wire_values(&self) -> Vec<String> {
        let mut values = Vec::new();
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
        values.extend(
            self.env_read
                .iter()
                .map(|name| PluginHostGrant::EnvRead { name: name.clone() }.to_wire()),
        );
        values
    }

    pub fn is_empty(&self) -> bool {
        !self.payload_read_all
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
            && self.env_read.is_empty()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PluginHostGrant {
    PayloadRead,
    PayloadReadSource {
        source: String,
    },
    ContextQuery,
    TraceAnalysisRead,
    TraceActivityRead,
    TraceFileStateRead,
    AlertWrite,
    FileAccessCurrentMatchGet,
    FileAccessCurrentContextQuery,
    FilePolicyRulesRead,
    FilePolicyRulesMatchDryRun,
    FilePolicyRulesValidate,
    FilePolicyRulesApply {
        decision: FilePolicyDecision,
        path: String,
    },
    CommandExecutionCurrentContextQuery,
    CommandPolicyRulesRead,
    CommandPolicyRulesMatchDryRun,
    CommandPolicyRulesValidate,
    CommandPolicyRulesApply {
        decision: CommandPolicyDecision,
        path: String,
    },
    EnvRead {
        name: String,
    },
}

impl PluginHostGrant {
    pub fn parse(raw: &str) -> Result<Self, String> {
        if raw == "payload-read" {
            return Ok(Self::PayloadRead);
        }
        if raw == "context-query" {
            return Ok(Self::ContextQuery);
        }
        if raw == "trace-analysis-read" {
            return Ok(Self::TraceAnalysisRead);
        }
        if raw == "trace-activity-read" {
            return Ok(Self::TraceActivityRead);
        }
        if raw == "trace-file-state-read" {
            return Ok(Self::TraceFileStateRead);
        }
        if raw == "alert-write" {
            return Ok(Self::AlertWrite);
        }
        if raw == "file-access.current-match-get" {
            return Ok(Self::FileAccessCurrentMatchGet);
        }
        if raw == "file-access.current-context-query" {
            return Ok(Self::FileAccessCurrentContextQuery);
        }
        if raw == "file-policy.rules.read" {
            return Ok(Self::FilePolicyRulesRead);
        }
        if raw == "file-policy.rules.match-dry-run" {
            return Ok(Self::FilePolicyRulesMatchDryRun);
        }
        if raw == "file-policy.rules.validate" {
            return Ok(Self::FilePolicyRulesValidate);
        }
        if raw == "command-execution.current-context-query" {
            return Ok(Self::CommandExecutionCurrentContextQuery);
        }
        if raw == "command-policy.rules.read" {
            return Ok(Self::CommandPolicyRulesRead);
        }
        if raw == "command-policy.rules.match-dry-run" {
            return Ok(Self::CommandPolicyRulesMatchDryRun);
        }
        if raw == "command-policy.rules.validate" {
            return Ok(Self::CommandPolicyRulesValidate);
        }
        let Some((kind, value)) = raw.split_once(':') else {
            return Err(format!(
                "invalid plugin host grant {raw}; expected a declared unscoped grant, payload-read:source=<boundary>, file-policy.rules.apply:kind=<allow|deny|gray>,path=<absolute|/absolute/**>, command-policy.rules.apply:kind=<allow|deny|gray>,path=<absolute|/absolute/**>, or env-read:NAME"
            ));
        };
        match kind {
            "payload-read" => {
                let Some(source) = value.strip_prefix("source=") else {
                    return Err(format!(
                        "invalid plugin host grant {raw}; expected payload-read:source=syscall, payload-read:source=tls-user-space, or payload-read:source=stdio"
                    ));
                };
                GrantValidator::payload_source(source)?;
                Ok(Self::PayloadReadSource {
                    source: source.to_string(),
                })
            }
            "env-read" => {
                GrantValidator::env_name(value)?;
                Ok(Self::EnvRead {
                    name: value.to_string(),
                })
            }
            "file-policy.rules.apply" => ScopedApplyGrantParser::file(value),
            "command-policy.rules.apply" => ScopedApplyGrantParser::command(value),
            other => Err(format!(
                "unsupported plugin host grant {other}; command policy grants use command-policy.rules.read, command-policy.rules.match-dry-run, command-policy.rules.validate, or command-policy.rules.apply:kind=<allow|deny|gray>,path=<absolute|/absolute/**>"
            )),
        }
    }

    pub fn capability(&self) -> PluginCapability {
        match self {
            Self::PayloadRead | Self::PayloadReadSource { .. } => PluginCapability::PayloadRead,
            Self::ContextQuery => PluginCapability::ContextQuery,
            Self::TraceAnalysisRead => PluginCapability::TraceAnalysisRead,
            Self::TraceActivityRead => PluginCapability::TraceActivityRead,
            Self::TraceFileStateRead => PluginCapability::TraceFileStateRead,
            Self::AlertWrite => PluginCapability::AlertWrite,
            Self::FileAccessCurrentMatchGet => PluginCapability::FileAccessCurrentMatchGet,
            Self::FileAccessCurrentContextQuery => PluginCapability::FileAccessCurrentContextQuery,
            Self::FilePolicyRulesRead => PluginCapability::FilePolicyRulesRead,
            Self::FilePolicyRulesMatchDryRun => PluginCapability::FilePolicyRulesMatchDryRun,
            Self::FilePolicyRulesValidate => PluginCapability::FilePolicyRulesValidate,
            Self::FilePolicyRulesApply { .. } => PluginCapability::FilePolicyRulesApply,
            Self::CommandExecutionCurrentContextQuery => {
                PluginCapability::CommandExecutionCurrentContextQuery
            }
            Self::CommandPolicyRulesRead => PluginCapability::CommandPolicyRulesRead,
            Self::CommandPolicyRulesMatchDryRun => PluginCapability::CommandPolicyRulesMatchDryRun,
            Self::CommandPolicyRulesValidate => PluginCapability::CommandPolicyRulesValidate,
            Self::CommandPolicyRulesApply { .. } => PluginCapability::CommandPolicyRulesApply,
            Self::EnvRead { .. } => PluginCapability::EnvRead,
        }
    }

    pub fn to_wire(&self) -> String {
        match self {
            Self::PayloadRead => "payload-read".to_string(),
            Self::PayloadReadSource { source } => format!("payload-read:source={source}"),
            Self::ContextQuery => "context-query".to_string(),
            Self::TraceAnalysisRead => "trace-analysis-read".to_string(),
            Self::TraceActivityRead => "trace-activity-read".to_string(),
            Self::TraceFileStateRead => "trace-file-state-read".to_string(),
            Self::AlertWrite => "alert-write".to_string(),
            Self::FileAccessCurrentMatchGet => "file-access.current-match-get".to_string(),
            Self::FileAccessCurrentContextQuery => "file-access.current-context-query".to_string(),
            Self::FilePolicyRulesRead => "file-policy.rules.read".to_string(),
            Self::FilePolicyRulesMatchDryRun => "file-policy.rules.match-dry-run".to_string(),
            Self::FilePolicyRulesValidate => "file-policy.rules.validate".to_string(),
            Self::FilePolicyRulesApply { decision, path } => format!(
                "file-policy.rules.apply:kind={},path={}",
                decision.as_str(),
                path
            ),
            Self::CommandExecutionCurrentContextQuery => {
                "command-execution.current-context-query".to_string()
            }
            Self::CommandPolicyRulesRead => "command-policy.rules.read".to_string(),
            Self::CommandPolicyRulesMatchDryRun => "command-policy.rules.match-dry-run".to_string(),
            Self::CommandPolicyRulesValidate => "command-policy.rules.validate".to_string(),
            Self::CommandPolicyRulesApply { decision, path } => format!(
                "command-policy.rules.apply:kind={},path={}",
                decision.as_str(),
                path
            ),
            Self::EnvRead { name } => format!("env-read:{name}"),
        }
    }
}
