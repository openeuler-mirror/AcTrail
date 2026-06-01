//! Shared key-value parsing helpers for operator config files.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::str::FromStr;

use model_core::capability::{Capability, CapabilityRequest, RequestMode};

use super::MemlockRlimit;

pub(super) struct ConfigValues {
    values: BTreeMap<String, Vec<String>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ConfigNode {
    name: String,
    values: BTreeMap<String, Vec<String>>,
}

impl ConfigValues {
    pub(super) fn parse(raw: &str) -> Result<Self, String> {
        let repeated = repeated_keys();
        let mut values = BTreeMap::<String, Vec<String>>::new();
        for (line_index, line) in raw.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let (key, value) = trimmed
                .split_once('=')
                .ok_or_else(|| format!("invalid config line {}", line_index + 1))?;
            let key = key.trim().to_string();
            let value = unquote(value.trim())?;
            if !repeated.contains(key.as_str()) && values.contains_key(&key) {
                return Err(format!("duplicate config key {key}"));
            }
            values.entry(key).or_default().push(value);
        }
        Ok(Self { values })
    }

    pub(super) fn node(&self, name: &'static str) -> ConfigNode {
        let prefix = format!("{name}_");
        ConfigNode {
            name: name.to_string(),
            values: self
                .values
                .iter()
                .filter_map(|(key, values)| {
                    key.strip_prefix(&prefix)
                        .map(|stripped| (stripped.to_string(), values.clone()))
                })
                .collect(),
        }
    }

    pub(super) fn required(&self, key: &'static str) -> Result<String, String> {
        let values = self
            .values
            .get(key)
            .filter(|values| !values.is_empty())
            .ok_or_else(|| format!("missing config key {key}"))?;
        if values.len() != 1 {
            return Err(format!("config key {key} must appear once"));
        }
        values
            .first()
            .cloned()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("config key {key} must not be empty"))
    }

    pub(super) fn required_bool(&self, key: &'static str) -> Result<bool, String> {
        match self.required(key)?.as_str() {
            "true" => Ok(true),
            "false" => Ok(false),
            value => Err(format!(
                "invalid {key}: expected true or false, got {value}"
            )),
        }
    }

    pub(super) fn required_u32(&self, key: &'static str) -> Result<u32, String> {
        self.required(key)?
            .parse::<u32>()
            .map_err(|error| format!("invalid {key}: {error}"))
    }

    pub(super) fn required_positive_u32(&self, key: &'static str) -> Result<u32, String> {
        let value = self.required_u32(key)?;
        if value == u32::default() {
            return Err(format!("invalid {key}: value must be positive"));
        }
        Ok(value)
    }

    pub(super) fn required_u64(&self, key: &'static str) -> Result<u64, String> {
        self.required(key)?
            .parse::<u64>()
            .map_err(|error| format!("invalid {key}: {error}"))
    }

    pub(super) fn required_positive_u64(&self, key: &'static str) -> Result<u64, String> {
        let value = self.required_u64(key)?;
        if value == u64::default() {
            return Err(format!("invalid {key}: value must be positive"));
        }
        Ok(value)
    }

    pub(super) fn required_octal(&self, key: &'static str) -> Result<u32, String> {
        u32::from_str_radix(&self.required(key)?, 8)
            .map_err(|error| format!("invalid {key}: {error}"))
    }

    pub(super) fn required_memlock_rlimit(
        &self,
        key: &'static str,
    ) -> Result<MemlockRlimit, String> {
        self.required(key)?
            .parse::<MemlockRlimit>()
            .map_err(|error| format!("invalid {key}: {error}"))
    }

    pub(super) fn capability_requests(&self) -> Result<Vec<CapabilityRequest>, String> {
        let mut requests = Vec::new();
        for raw in self.repeated("required_capability") {
            requests.push(CapabilityRequest::new(
                parse_capability(raw)?,
                RequestMode::Required,
            ));
        }
        for raw in self.repeated("opportunistic_capability") {
            requests.push(CapabilityRequest::new(
                parse_capability(raw)?,
                RequestMode::Opportunistic,
            ));
        }
        for raw in self.repeated("disabled_capability") {
            requests.push(CapabilityRequest::new(
                parse_capability(raw)?,
                RequestMode::Disabled,
            ));
        }
        Ok(requests)
    }

    fn repeated(&self, key: &'static str) -> impl Iterator<Item = &String> {
        self.values.get(key).into_iter().flatten()
    }
}

impl ConfigNode {
    pub(super) fn required(&self, key: &'static str) -> Result<String, String> {
        let values = self
            .values
            .get(key)
            .filter(|values| !values.is_empty())
            .ok_or_else(|| format!("missing config key {}", self.qualified_key(key)))?;
        if values.len() != 1 {
            return Err(format!(
                "config key {} must appear once",
                self.qualified_key(key)
            ));
        }
        values
            .first()
            .cloned()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("config key {} must not be empty", self.qualified_key(key)))
    }

    pub(super) fn required_bool(&self, key: &'static str) -> Result<bool, String> {
        match self.required(key)?.as_str() {
            "true" => Ok(true),
            "false" => Ok(false),
            value => Err(format!(
                "invalid {}: expected true or false, got {value}",
                self.qualified_key(key)
            )),
        }
    }

    pub(super) fn optional_bool(&self, key: &'static str, default: bool) -> Result<bool, String> {
        let Some(values) = self.values.get(key) else {
            return Ok(default);
        };
        if values.len() != 1 {
            return Err(format!(
                "config key {} must appear once",
                self.qualified_key(key)
            ));
        }
        match values
            .first()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("config key {} must not be empty", self.qualified_key(key)))?
            .as_str()
        {
            "true" => Ok(true),
            "false" => Ok(false),
            value => Err(format!(
                "invalid {}: expected true or false, got {value}",
                self.qualified_key(key)
            )),
        }
    }

    pub(super) fn required_u32(&self, key: &'static str) -> Result<u32, String> {
        self.required(key)?
            .parse::<u32>()
            .map_err(|error| format!("invalid {}: {error}", self.qualified_key(key)))
    }

    pub(super) fn required_positive_u32(&self, key: &'static str) -> Result<u32, String> {
        let value = self.required_u32(key)?;
        if value == u32::default() {
            return Err(format!(
                "invalid {}: value must be positive",
                self.qualified_key(key)
            ));
        }
        Ok(value)
    }

    pub(super) fn required_u64(&self, key: &'static str) -> Result<u64, String> {
        self.required(key)?
            .parse::<u64>()
            .map_err(|error| format!("invalid {}: {error}", self.qualified_key(key)))
    }

    pub(super) fn required_positive_u64(&self, key: &'static str) -> Result<u64, String> {
        let value = self.required_u64(key)?;
        if value == u64::default() {
            return Err(format!(
                "invalid {}: value must be positive",
                self.qualified_key(key)
            ));
        }
        Ok(value)
    }

    pub(super) fn required_disabled_or_positive_u64(
        &self,
        key: &'static str,
    ) -> Result<Option<u64>, String> {
        let raw = self.required(key)?;
        if raw == "disabled" {
            return Ok(None);
        }
        raw.parse::<u64>()
            .map_err(|error| format!("invalid {}: {error}", self.qualified_key(key)))
            .and_then(|value| {
                if value == u64::default() {
                    Err(format!(
                        "invalid {}: value must be positive or disabled",
                        self.qualified_key(key)
                    ))
                } else {
                    Ok(Some(value))
                }
            })
    }

    pub(super) fn required_path_buf(&self, key: &'static str) -> Result<PathBuf, String> {
        self.required(key).map(PathBuf::from)
    }

    pub(super) fn required_parsed<T>(&self, key: &'static str) -> Result<T, String>
    where
        T: FromStr<Err = String>,
    {
        self.required(key)?
            .parse::<T>()
            .map_err(|error| format!("invalid {}: {error}", self.qualified_key(key)))
    }

    pub(super) fn repeated_parsed<T>(&self, key: &'static str) -> Result<Vec<T>, String>
    where
        T: FromStr<Err = String>,
    {
        let values = self
            .values
            .get(key)
            .filter(|values| !values.is_empty())
            .ok_or_else(|| format!("missing config key {}", self.qualified_key(key)))?;
        values
            .iter()
            .map(|raw| {
                raw.parse::<T>()
                    .map_err(|error| format!("invalid {}: {error}", self.qualified_key(key)))
            })
            .collect()
    }

    pub(super) fn repeated(
        &self,
        key: &'static str,
    ) -> Result<impl Iterator<Item = &String>, String> {
        self.values
            .get(key)
            .filter(|values| !values.is_empty())
            .map(|values| values.iter())
            .ok_or_else(|| format!("missing config key {}", self.qualified_key(key)))
    }

    fn qualified_key(&self, key: &'static str) -> String {
        format!("{}_{}", self.name, key)
    }
}

fn repeated_keys() -> BTreeSet<&'static str> {
    [
        "required_capability",
        "opportunistic_capability",
        "disabled_capability",
        "payload_tls_seccomp_syscall",
        "payload_socket_seccomp_syscall",
        "process_seccomp_syscall",
        "agent_invocation_command",
    ]
    .into_iter()
    .collect()
}

fn unquote(value: &str) -> Result<String, String> {
    if value.starts_with('"') || value.ends_with('"') {
        if !(value.starts_with('"') && value.ends_with('"') && value.len() >= 2) {
            return Err(format!("invalid quoted value {value}"));
        }
        return Ok(value[1..value.len() - 1].to_string());
    }
    Ok(value.to_string())
}

fn parse_capability(raw: &str) -> Result<Capability, String> {
    match raw {
        "proc-lifecycle" => Ok(Capability::ProcLifecycle),
        "proc-exec-context" => Ok(Capability::ProcExecContext),
        "fs-access-basic" => Ok(Capability::FsAccessBasic),
        "fs-mmap" => Ok(Capability::FsMmap),
        "fs-exec-access" => Ok(Capability::FsExecAccess),
        "net-transport" => Ok(Capability::NetTransport),
        "net-dns" => Ok(Capability::NetDns),
        "net-tls-metadata" => Ok(Capability::NetTlsMetadata),
        "net-provider-classification" => Ok(Capability::NetProviderClassification),
        "net-application-plaintext-http" => Ok(Capability::NetApplicationPlaintextHttp),
        "net-application-http2-frames" => Ok(Capability::NetApplicationHttp2Frames),
        "net-application-plaintext-ws" => Ok(Capability::NetApplicationPlaintextWs),
        "tls-plaintext-payload" => Ok(Capability::TlsPlaintextPayload),
        "socket-plaintext-payload" => Ok(Capability::SocketPlaintextPayload),
        "resource-metrics" => Ok(Capability::ResourceMetrics),
        "ipc-unix-socket" => Ok(Capability::IpcUnixSocket),
        "ipc-pipe-fifo" => Ok(Capability::IpcPipeFifo),
        "stdio-chunk" => Ok(Capability::StdioChunk),
        "policy-ingest-processing" => Ok(Capability::PolicyIngestProcessing),
        "policy-plugin-host" => Ok(Capability::PolicyPluginHost),
        "policy-decision-record" => Ok(Capability::PolicyDecisionRecord),
        "enforcement-file-permission-fanotify" => Ok(Capability::EnforcementFilePermissionFanotify),
        other => Err(format!("unknown capability {other}")),
    }
}
