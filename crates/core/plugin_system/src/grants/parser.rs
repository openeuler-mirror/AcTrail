use crate::{CommandPolicyDecision, FilePolicyDecision};

use super::model::PluginHostGrant;
use super::validation::GrantValidator;

pub(super) struct ScopedApplyGrantParser;

impl ScopedApplyGrantParser {
    pub(super) fn file(value: &str) -> Result<PluginHostGrant, String> {
        let (decision, path) = Self::fields(value, "file-policy.rules.apply")?;
        let decision = FilePolicyDecision::from_wire(&decision)?;
        GrantValidator::file_decision(decision)?;
        GrantValidator::file_path_scope(&path)?;
        Ok(PluginHostGrant::FilePolicyRulesApply { decision, path })
    }

    pub(super) fn command(value: &str) -> Result<PluginHostGrant, String> {
        let (decision, path) = Self::fields(value, "command-policy.rules.apply")?;
        let decision = CommandPolicyDecision::from_wire(&decision)?;
        GrantValidator::command_decision(decision)?;
        GrantValidator::command_path_scope(&path)?;
        Ok(PluginHostGrant::CommandPolicyRulesApply { decision, path })
    }

    fn fields(value: &str, label: &str) -> Result<(String, String), String> {
        let mut decision = None;
        let mut path = None;
        for part in value.split(',') {
            let Some((key, raw_value)) = part.split_once('=') else {
                return Err(format!(
                    "invalid {label} grant segment {part}; expected key=value"
                ));
            };
            match key {
                "kind" if decision.is_none() => decision = Some(raw_value.to_string()),
                "path" if path.is_none() => path = Some(raw_value.to_string()),
                "kind" | "path" => return Err(format!("{label} grant repeats {key}")),
                other => {
                    return Err(format!(
                        "unsupported {label} grant key {other}; expected kind or path"
                    ));
                }
            }
        }
        Ok((
            decision.ok_or_else(|| format!("{label} grant requires kind=allow|deny|gray"))?,
            path.ok_or_else(|| {
                format!("{label} grant requires path=/absolute/path or path=/absolute/**")
            })?,
        ))
    }
}
