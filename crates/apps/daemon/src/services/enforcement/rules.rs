//! Exact-path access-control rule loading.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use config_core::daemon::EnforcementDecision;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct FileKey {
    pub dev: u64,
    pub ino: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct EnforcementRule {
    pub rule_id: String,
    pub decision: EnforcementDecision,
    pub operation: String,
    pub path: PathBuf,
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
}

fn parse_rule(raw: &str) -> Result<EnforcementRule, String> {
    let fields = raw.split_whitespace().collect::<Vec<_>>();
    let [rule_id, decision, operation, path] = fields.as_slice() else {
        return Err("expected: <rule_id> <allow|deny> open <absolute-path>".to_string());
    };
    if *operation != "open" {
        return Err(format!("unsupported operation {operation}; expected open"));
    }
    let path = PathBuf::from(path);
    if !path.is_absolute() {
        return Err(format!("path {} must be absolute", path.display()));
    }
    let path = fs::canonicalize(&path)
        .map_err(|error| format!("canonicalize {}: {error}", path.display()))?;
    Ok(EnforcementRule {
        rule_id: (*rule_id).to_string(),
        decision: (*decision).parse::<EnforcementDecision>()?,
        operation: (*operation).to_string(),
        path,
    })
}
