//! Individual host grants and their wire representation.
use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PluginHostGrant {
    HttpRequest,
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
    NetworkActionCurrentContextQuery,
    NetworkPolicyRulesRead,
    NetworkPolicyRulesMatchDryRun,
    NetworkPolicyRulesValidate,
    NetworkPolicyRulesApply {
        decision: NetworkPolicyDecision,
        operation: NetworkPolicyOperation,
        protocol: NetworkPolicyProtocol,
        endpoint: String,
    },
    EnvRead {
        name: String,
    },
}

impl PluginHostGrant {
    pub fn parse(raw: &str) -> Result<Self, String> {
        if raw == "http.request" {
            return Ok(Self::HttpRequest);
        }
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
        if raw == "network-action.current-context-query" {
            return Ok(Self::NetworkActionCurrentContextQuery);
        }
        if raw == "network-policy.rules.read" {
            return Ok(Self::NetworkPolicyRulesRead);
        }
        if raw == "network-policy.rules.match-dry-run" {
            return Ok(Self::NetworkPolicyRulesMatchDryRun);
        }
        if raw == "network-policy.rules.validate" {
            return Ok(Self::NetworkPolicyRulesValidate);
        }
        let Some((kind, value)) = raw.split_once(':') else {
            return Err(format!(
                "invalid plugin host grant {raw}; expected a declared unscoped grant, payload-read:source=<boundary>, file-policy.rules.apply:kind=<allow|deny|gray>,path=<absolute|/absolute/**>, command-policy.rules.apply:kind=<allow|deny|gray>,path=<absolute|/absolute/**>, network-policy.rules.apply:kind=<allow|deny|gray>,operation=<connect|send|bind|listen>,protocol=<any|tcp|udp>,endpoint=<*|ip-or-cidr:port-or-range-or-*>, or env-read:NAME"
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
            "network-policy.rules.apply" => ScopedApplyGrantParser::network(value),
            other => Err(format!(
                "unsupported plugin host grant {other}; managed policy grants use their declared read, match-dry-run, validate, or scoped apply form"
            )),
        }
    }

    pub fn capability(&self) -> PluginCapability {
        match self {
            Self::HttpRequest => PluginCapability::HttpRequest,
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
            Self::NetworkActionCurrentContextQuery => {
                PluginCapability::NetworkActionCurrentContextQuery
            }
            Self::NetworkPolicyRulesRead => PluginCapability::NetworkPolicyRulesRead,
            Self::NetworkPolicyRulesMatchDryRun => PluginCapability::NetworkPolicyRulesMatchDryRun,
            Self::NetworkPolicyRulesValidate => PluginCapability::NetworkPolicyRulesValidate,
            Self::NetworkPolicyRulesApply { .. } => PluginCapability::NetworkPolicyRulesApply,
            Self::EnvRead { .. } => PluginCapability::EnvRead,
        }
    }

    pub fn to_wire(&self) -> String {
        match self {
            Self::HttpRequest => "http.request".to_string(),
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
            Self::NetworkActionCurrentContextQuery => {
                "network-action.current-context-query".to_string()
            }
            Self::NetworkPolicyRulesRead => "network-policy.rules.read".to_string(),
            Self::NetworkPolicyRulesMatchDryRun => "network-policy.rules.match-dry-run".to_string(),
            Self::NetworkPolicyRulesValidate => "network-policy.rules.validate".to_string(),
            Self::NetworkPolicyRulesApply {
                decision,
                operation,
                protocol,
                endpoint,
            } => format!(
                "network-policy.rules.apply:kind={},operation={},protocol={},endpoint={}",
                decision.as_str(),
                operation.as_str(),
                protocol.as_str(),
                endpoint
            ),
            Self::EnvRead { name } => format!("env-read:{name}"),
        }
    }
}
