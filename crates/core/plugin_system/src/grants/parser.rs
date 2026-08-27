use crate::{CommandPolicyDecision, FilePolicyDecision, NetworkPolicyDecision};

use super::model::PluginHostGrant;
use super::validation::GrantValidator;

pub(super) struct ScopedApplyGrantParser;

impl ScopedApplyGrantParser {
    pub(super) fn file(value: &str) -> Result<PluginHostGrant, String> {
        let (decision, path) = Self::fields(
            value,
            "file-policy.rules.apply",
            "path",
            "/absolute/path or /absolute/**",
        )?;
        let decision = FilePolicyDecision::from_wire(&decision)?;
        GrantValidator::file_decision(decision)?;
        GrantValidator::file_path_scope(&path)?;
        Ok(PluginHostGrant::FilePolicyRulesApply { decision, path })
    }

    pub(super) fn command(value: &str) -> Result<PluginHostGrant, String> {
        let (decision, path) = Self::fields(
            value,
            "command-policy.rules.apply",
            "path",
            "/absolute/path or /absolute/**",
        )?;
        let decision = CommandPolicyDecision::from_wire(&decision)?;
        GrantValidator::command_decision(decision)?;
        GrantValidator::command_path_scope(&path)?;
        Ok(PluginHostGrant::CommandPolicyRulesApply { decision, path })
    }

    pub(super) fn network(value: &str) -> Result<PluginHostGrant, String> {
        let (decision, remote) = Self::fields(
            value,
            "network-policy.rules.apply",
            "remote",
            "*, a numeric IP endpoint, or an IP any-port selector",
        )?;
        let decision = NetworkPolicyDecision::from_wire(&decision)?;
        GrantValidator::network_decision(decision)?;
        let remote = GrantValidator::network_remote_scope(&remote)?.to_string();
        Ok(PluginHostGrant::NetworkPolicyRulesApply { decision, remote })
    }

    fn fields(
        value: &str,
        label: &str,
        scope_key: &str,
        scope_help: &str,
    ) -> Result<(String, String), String> {
        let mut decision = None;
        let mut scope = None;
        for part in value.split(',') {
            let Some((key, raw_value)) = part.split_once('=') else {
                return Err(format!(
                    "invalid {label} grant segment {part}; expected key=value"
                ));
            };
            match key {
                "kind" if decision.is_none() => decision = Some(raw_value.to_string()),
                key if key == scope_key && scope.is_none() => scope = Some(raw_value.to_string()),
                "kind" => return Err(format!("{label} grant repeats {key}")),
                key if key == scope_key => return Err(format!("{label} grant repeats {key}")),
                other => {
                    return Err(format!(
                        "unsupported {label} grant key {other}; expected kind or {scope_key}"
                    ));
                }
            }
        }
        Ok((
            decision.ok_or_else(|| format!("{label} grant requires kind=allow|deny|gray"))?,
            scope.ok_or_else(|| format!("{label} grant requires {scope_key}={scope_help}"))?,
        ))
    }
}
