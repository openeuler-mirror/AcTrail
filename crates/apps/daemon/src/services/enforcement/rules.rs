//! Exact-path access-control rule loading.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use config_core::daemon::EnforcementDecision;
use plugin_system::FilePolicyWriteUpdate;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct FileKey {
    pub dev: u64,
    pub ino: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct EnforcementRule {
    pub rule_id: String,
    pub decision: RuleDecision,
    pub operation: String,
    pub path: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum RuleDecision {
    Local(EnforcementDecision),
    SyncPlugin {
        instance_id: String,
        timeout_ms: u64,
        concurrency_limit: u32,
        fallback: EnforcementDecision,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct EnforcementRules {
    by_path: BTreeMap<PathBuf, EnforcementRule>,
    mark_directories: BTreeSet<PathBuf>,
}

impl EnforcementRules {
    pub(super) fn load(path: &Path) -> Result<Self, String> {
        let raw = fs::read_to_string(path)
            .map_err(|error| format!("read enforcement rules {}: {error}", path.display()))?;
        Self::parse(&raw)
    }

    pub(super) fn parse(raw: &str) -> Result<Self, String> {
        let mut by_path = BTreeMap::new();
        let mut rule_ids = BTreeSet::new();
        let mut mark_directories = BTreeSet::new();
        for (line_index, line) in raw.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let rule = parse_rule(trimmed)
                .map_err(|error| format!("enforcement rule line {}: {error}", line_index + 1))?;
            let parent = rule
                .path
                .parent()
                .ok_or_else(|| format!("enforcement rule {} has no parent path", rule.rule_id))?
                .to_path_buf();
            let rule_id = rule.rule_id.clone();
            if !rule_ids.insert(rule_id.clone()) {
                return Err(format!(
                    "enforcement rule line {} duplicates rule id {}",
                    line_index + 1,
                    rule_id
                ));
            }
            if by_path.insert(rule.path.clone(), rule).is_some() {
                return Err(format!(
                    "enforcement rule line {} duplicates a path",
                    line_index + 1
                ));
            }
            mark_directories.insert(parent);
        }
        Ok(Self {
            by_path,
            mark_directories,
        })
    }

    pub(super) fn find_path(&self, path: &Path) -> Option<&EnforcementRule> {
        self.by_path.get(path)
    }

    pub(super) fn is_empty(&self) -> bool {
        self.by_path.is_empty()
    }

    pub(super) fn mark_directories(&self) -> impl Iterator<Item = &PathBuf> {
        self.mark_directories.iter()
    }

    pub(super) fn apply_file_policy_update(
        &mut self,
        update: FilePolicyWriteUpdate,
    ) -> Result<Option<PathBuf>, String> {
        validate_operation(&update.operation)?;
        let path = canonical_path(&update.path)?;
        let parent = path
            .parent()
            .ok_or_else(|| format!("enforcement rule {} has no parent path", update.rule_id))?
            .to_path_buf();
        if update.rule_id.trim().is_empty() {
            return Err("file-policy update rule_id must not be empty".to_string());
        }
        if self
            .by_path
            .values()
            .any(|rule| rule.rule_id == update.rule_id && rule.path != path)
        {
            return Err(format!(
                "file-policy update duplicates rule id {} for a different path",
                update.rule_id
            ));
        }
        let rule = EnforcementRule {
            rule_id: update.rule_id,
            decision: RuleDecision::Local(update.decision.parse::<EnforcementDecision>()?),
            operation: update.operation,
            path: path.clone(),
        };
        self.by_path.insert(path, rule);
        Ok(self
            .mark_directories
            .insert(parent.clone())
            .then_some(parent))
    }
}

fn parse_rule(raw: &str) -> Result<EnforcementRule, String> {
    let fields = raw.split_whitespace().collect::<Vec<_>>();
    match fields.as_slice() {
        [rule_id, decision, operation, path] => {
            parse_local_rule(rule_id, decision, operation, path)
        }
        [
            rule_id,
            "gray",
            "sync-plugin",
            instance_id,
            "timeout-ms",
            timeout_ms,
            "concurrency",
            concurrency_limit,
            "fallback",
            fallback,
            operation,
            path,
        ] => parse_sync_plugin_rule(
            rule_id,
            instance_id,
            timeout_ms,
            concurrency_limit,
            fallback,
            operation,
            path,
        ),
        _ => Err(
            "expected: <rule_id> <allow|deny> open <absolute-path> or <rule_id> gray sync-plugin <instance> timeout-ms <positive-ms> concurrency <positive-limit> fallback <allow|deny> open <absolute-path>"
                .to_string(),
        ),
    }
}

fn parse_local_rule(
    rule_id: &str,
    decision: &str,
    operation: &str,
    path: &str,
) -> Result<EnforcementRule, String> {
    validate_operation(operation)?;
    Ok(EnforcementRule {
        rule_id: rule_id.to_string(),
        decision: RuleDecision::Local(decision.parse::<EnforcementDecision>()?),
        operation: operation.to_string(),
        path: canonical_path(path)?,
    })
}

fn parse_sync_plugin_rule(
    rule_id: &str,
    instance_id: &str,
    timeout_ms: &str,
    concurrency_limit: &str,
    fallback: &str,
    operation: &str,
    path: &str,
) -> Result<EnforcementRule, String> {
    validate_operation(operation)?;
    if instance_id.trim().is_empty() {
        return Err("sync-plugin instance id must not be empty".to_string());
    }
    let timeout_ms = parse_positive_u64("timeout-ms", timeout_ms)?;
    let concurrency_limit = parse_positive_u32("concurrency", concurrency_limit)?;
    Ok(EnforcementRule {
        rule_id: rule_id.to_string(),
        decision: RuleDecision::SyncPlugin {
            instance_id: instance_id.to_string(),
            timeout_ms,
            concurrency_limit,
            fallback: fallback.parse::<EnforcementDecision>()?,
        },
        operation: operation.to_string(),
        path: canonical_path(path)?,
    })
}

fn validate_operation(operation: &str) -> Result<(), String> {
    if operation != "open" {
        return Err(format!("unsupported operation {operation}; expected open"));
    }
    Ok(())
}

fn parse_positive_u64(name: &str, raw: &str) -> Result<u64, String> {
    let value = raw
        .parse::<u64>()
        .map_err(|error| format!("{name} must be a positive integer: {error}"))?;
    if value == 0 {
        return Err(format!("{name} must be greater than zero"));
    }
    Ok(value)
}

fn parse_positive_u32(name: &str, raw: &str) -> Result<u32, String> {
    let value = raw
        .parse::<u32>()
        .map_err(|error| format!("{name} must be a positive integer: {error}"))?;
    if value == 0 {
        return Err(format!("{name} must be greater than zero"));
    }
    Ok(value)
}

fn canonical_path(path: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(path);
    if !path.is_absolute() {
        return Err(format!("path {} must be absolute", path.display()));
    }
    fs::canonicalize(&path).map_err(|error| format!("canonicalize {}: {error}", path.display()))
}
