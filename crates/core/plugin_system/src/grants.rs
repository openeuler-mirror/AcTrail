use std::collections::BTreeSet;

use model_core::payload::PayloadSourceBoundary;

use crate::PluginCapability;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PluginHostGrants {
    payload_read_all: bool,
    payload_read_sources: BTreeSet<String>,
    context_query: bool,
    file_policy_read: bool,
    file_policy_write: bool,
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
                PluginHostGrant::PayloadRead => grants.allow_payload_read(),
                PluginHostGrant::PayloadReadSource { source } => {
                    grants.allow_payload_read_source(source)?
                }
                PluginHostGrant::ContextQuery => grants.allow_context_query(),
                PluginHostGrant::FilePolicyRead => grants.allow_file_policy_read(),
                PluginHostGrant::FilePolicyWrite => grants.allow_file_policy_write(),
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
        validate_payload_source_boundary(&source)?;
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
                .contains(payload_source_boundary_grant_name(source_boundary))
    }

    pub fn allow_env_read(&mut self, name: impl Into<String>) -> Result<(), String> {
        let name = name.into();
        validate_env_name(&name)?;
        self.env_read.insert(name);
        Ok(())
    }

    pub fn allow_context_query(&mut self) {
        self.context_query = true;
    }

    pub fn can_query_context(&self) -> bool {
        self.context_query
    }

    pub fn allow_file_policy_read(&mut self) {
        self.file_policy_read = true;
    }

    pub fn can_read_file_policy(&self) -> bool {
        self.file_policy_read
    }

    pub fn allow_file_policy_write(&mut self) {
        self.file_policy_write = true;
    }

    pub fn can_write_file_policy(&self) -> bool {
        self.file_policy_write
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
        if self.file_policy_read {
            values.push(PluginHostGrant::FilePolicyRead.to_wire());
        }
        if self.file_policy_write {
            values.push(PluginHostGrant::FilePolicyWrite.to_wire());
        }
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
            && !self.file_policy_read
            && !self.file_policy_write
            && self.env_read.is_empty()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PluginHostGrant {
    PayloadRead,
    PayloadReadSource { source: String },
    ContextQuery,
    FilePolicyRead,
    FilePolicyWrite,
    EnvRead { name: String },
}

impl PluginHostGrant {
    pub fn parse(raw: &str) -> Result<Self, String> {
        if raw == "payload-read" {
            return Ok(Self::PayloadRead);
        }
        if raw == "context-query" {
            return Ok(Self::ContextQuery);
        }
        if raw == "file-policy-read" {
            return Ok(Self::FilePolicyRead);
        }
        if raw == "file-policy-write" {
            return Ok(Self::FilePolicyWrite);
        }
        let Some((kind, value)) = raw.split_once(':') else {
            return Err(format!(
                "invalid plugin host grant {raw}; expected payload-read, payload-read:source=syscall, payload-read:source=tls-user-space, payload-read:source=stdio, context-query, file-policy-read, file-policy-write, or env-read:NAME"
            ));
        };
        match kind {
            "payload-read" => {
                let Some(source) = value.strip_prefix("source=") else {
                    return Err(format!(
                        "invalid plugin host grant {raw}; expected payload-read:source=syscall, payload-read:source=tls-user-space, or payload-read:source=stdio"
                    ));
                };
                validate_payload_source_boundary(source)?;
                Ok(Self::PayloadReadSource {
                    source: source.to_string(),
                })
            }
            "env-read" => {
                validate_env_name(value)?;
                Ok(Self::EnvRead {
                    name: value.to_string(),
                })
            }
            other => Err(format!(
                "unsupported plugin host grant {other}; supported grants: payload-read, payload-read:source=syscall, payload-read:source=tls-user-space, payload-read:source=stdio, context-query, file-policy-read, file-policy-write, env-read:NAME"
            )),
        }
    }

    pub fn capability(&self) -> PluginCapability {
        match self {
            Self::PayloadRead | Self::PayloadReadSource { .. } => PluginCapability::PayloadRead,
            Self::ContextQuery => PluginCapability::ContextQuery,
            Self::FilePolicyRead => PluginCapability::FilePolicyRead,
            Self::FilePolicyWrite => PluginCapability::FilePolicyWrite,
            Self::EnvRead { .. } => PluginCapability::EnvRead,
        }
    }

    pub fn to_wire(&self) -> String {
        match self {
            Self::PayloadRead => "payload-read".to_string(),
            Self::PayloadReadSource { source } => format!("payload-read:source={source}"),
            Self::ContextQuery => "context-query".to_string(),
            Self::FilePolicyRead => "file-policy-read".to_string(),
            Self::FilePolicyWrite => "file-policy-write".to_string(),
            Self::EnvRead { name } => format!("env-read:{name}"),
        }
    }
}

fn validate_env_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("env-read grant name must not be empty".to_string());
    }
    let mut chars = name.chars();
    let first = chars
        .next()
        .ok_or_else(|| "env-read grant name must not be empty".to_string())?;
    if !(first == '_' || first.is_ascii_alphabetic()) {
        return Err(format!(
            "env-read grant name {name} must start with an ASCII letter or underscore"
        ));
    }
    if chars.any(|ch| !(ch == '_' || ch.is_ascii_alphanumeric())) {
        return Err(format!(
            "env-read grant name {name} must contain only ASCII letters, digits, and underscores"
        ));
    }
    Ok(())
}

fn validate_payload_source_boundary(source: &str) -> Result<(), String> {
    match source {
        "syscall" | "tls-user-space" | "stdio" => Ok(()),
        _ => Err(format!(
            "unsupported payload-read source {source}; expected syscall, tls-user-space, or stdio"
        )),
    }
}

fn payload_source_boundary_grant_name(source_boundary: PayloadSourceBoundary) -> &'static str {
    match source_boundary {
        PayloadSourceBoundary::Syscall => "syscall",
        PayloadSourceBoundary::TlsUserSpace => "tls-user-space",
        PayloadSourceBoundary::Stdio => "stdio",
    }
}
