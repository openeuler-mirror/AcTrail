//! File policy data, operation conversion and configuration validation.
use super::actrail::plugin::types::{FilePolicyDecision, FilePolicyOperation};
use super::str_eq;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use serde::{Deserialize, Serialize};

#[derive(Clone, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PolicyConfig {
    #[serde(default)]
    pub(super) rules: Vec<PolicyRule>,
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PolicyRule {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) rule_id: Option<String>,
    pub(super) decision: PolicyDecision,
    #[serde(default)]
    pub(super) operation: PolicyOperation,
    pub(super) path: String,
    #[serde(default = "default_priority")]
    pub(super) priority: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) gray_target: Option<u64>,
}

#[derive(Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum PolicyDecision {
    Allow,
    Deny,
    Gray,
}

#[derive(Clone, Copy, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum PolicyOperation {
    #[default]
    Any,
    Open,
    Read,
    Write,
    Delete,
    Attr,
    Rename,
    Link,
    Mkdir,
    Rmdir,
}

impl PolicyConfig {
    pub(super) fn parse(raw: &str, current: &PolicyConfig) -> Result<PolicyConfig, String> {
        let mut config = serde_json::from_str::<PolicyConfig>(raw)
            .map_err(|error| format!("parse config JSON: {error}"))?;
        config.normalize_rule_ids(current)?;
        config.validate()?;
        Ok(config)
    }

    pub(super) fn normalize_rule_ids(&mut self, current: &PolicyConfig) -> Result<(), String> {
        let mut next_id = 1_u64;
        for rule in current.rules.iter().chain(self.rules.iter()) {
            if let Some(number) = rule
                .rule_id
                .as_deref()
                .and_then(|id| id.strip_prefix("dynamic-"))
                .and_then(|number| number.parse::<u64>().ok())
            {
                next_id = next_id.max(number.saturating_add(1));
            }
        }
        for rule in &mut self.rules {
            if rule
                .rule_id
                .as_deref()
                .is_none_or(|id| id.trim().is_empty())
            {
                rule.rule_id = Some(format!("dynamic-{next_id}"));
                next_id = next_id.saturating_add(1);
            }
        }
        self.validate()
    }

    fn validate(&self) -> Result<(), String> {
        for (index, rule) in self.rules.iter().enumerate() {
            let id = rule
                .rule_id
                .as_deref()
                .ok_or_else(|| format!("rules[{index}].rule_id is required"))?;
            if id.trim().is_empty() {
                return Err(format!("rules[{index}].rule_id must not be empty"));
            }
            if self.rules[..index]
                .iter()
                .any(|existing| existing.rule_id.as_deref() == Some(id))
            {
                return Err(format!("duplicate rule_id {id}"));
            }
            if rule.path.trim().is_empty() {
                return Err(format!("rules[{index}].path must not be empty"));
            }
            match rule.decision {
                PolicyDecision::Gray if rule.gray_target.is_none() => {
                    return Err(format!("rules[{index}].gray_target is required for gray"));
                }
                PolicyDecision::Allow | PolicyDecision::Deny if rule.gray_target.is_some() => {
                    return Err(format!("rules[{index}].gray_target is only valid for gray"));
                }
                _ => {}
            }
        }
        Ok(())
    }
}
impl PolicyDecision {
    pub(super) fn host_value(self) -> FilePolicyDecision {
        match self {
            Self::Allow => FilePolicyDecision::Allow,
            Self::Deny => FilePolicyDecision::Deny,
            Self::Gray => FilePolicyDecision::Gray,
        }
    }

    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Deny => "deny",
            Self::Gray => "gray",
        }
    }
}

impl PolicyOperation {
    pub(super) fn host_value(self) -> FilePolicyOperation {
        match self {
            Self::Any => FilePolicyOperation::Any,
            Self::Open => FilePolicyOperation::Open,
            Self::Read => FilePolicyOperation::Read,
            Self::Write => FilePolicyOperation::Write,
            Self::Delete => FilePolicyOperation::Delete,
            Self::Attr => FilePolicyOperation::Attr,
            Self::Rename => FilePolicyOperation::Rename,
            Self::Link => FilePolicyOperation::Link,
            Self::Mkdir => FilePolicyOperation::Mkdir,
            Self::Rmdir => FilePolicyOperation::Rmdir,
        }
    }

    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Any => "any",
            Self::Open => "open",
            Self::Read => "read",
            Self::Write => "write",
            Self::Delete => "delete",
            Self::Attr => "attr",
            Self::Rename => "rename",
            Self::Link => "link",
            Self::Mkdir => "mkdir",
            Self::Rmdir => "rmdir",
        }
    }
}

pub(super) fn parse_decision(value: &str) -> Result<PolicyDecision, String> {
    if str_eq(value, "allow") {
        Ok(PolicyDecision::Allow)
    } else if str_eq(value, "deny") {
        Ok(PolicyDecision::Deny)
    } else if str_eq(value, "gray") {
        Ok(PolicyDecision::Gray)
    } else {
        Err("decision must be allow, deny, or gray".to_string())
    }
}

pub(super) fn parse_operation(value: &str) -> Result<PolicyOperation, String> {
    if str_eq(value, "any") {
        Ok(PolicyOperation::Any)
    } else if str_eq(value, "open") {
        Ok(PolicyOperation::Open)
    } else if str_eq(value, "read") {
        Ok(PolicyOperation::Read)
    } else if str_eq(value, "write") {
        Ok(PolicyOperation::Write)
    } else if str_eq(value, "delete") {
        Ok(PolicyOperation::Delete)
    } else if str_eq(value, "attr") {
        Ok(PolicyOperation::Attr)
    } else if str_eq(value, "rename") {
        Ok(PolicyOperation::Rename)
    } else if str_eq(value, "link") {
        Ok(PolicyOperation::Link)
    } else if str_eq(value, "mkdir") {
        Ok(PolicyOperation::Mkdir)
    } else if str_eq(value, "rmdir") {
        Ok(PolicyOperation::Rmdir)
    } else {
        Err(
            "operation must be any, open, read, write, delete, attr, rename, link, mkdir, or rmdir"
                .to_string(),
        )
    }
}

pub(super) fn default_priority() -> i32 {
    10
}
