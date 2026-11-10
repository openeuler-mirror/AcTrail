use std::collections::BTreeSet;

use serde::Deserialize;
use toml::{Table, Value};

/// Immutable delivery and editing selection, validated against an owner-supplied template.
pub struct ConfigExportFilter {
    paths: BTreeSet<String>,
    mode: SelectionMode,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SelectionDocument {
    #[serde(default)]
    default: SelectionMode,
    whitelist: Vec<String>,
    #[serde(default)]
    blacklist: Vec<String>,
}

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
enum SelectionMode {
    #[default]
    Whitelist,
    Blacklist,
}

impl ConfigExportFilter {
    /// Load the release's selection policy. Defaults and field types belong to the caller.
    pub fn bundled(template: &Value) -> Result<Self, String> {
        const POLICY: Result<&str, &str> = include!(concat!(env!("OUT_DIR"), "/policy.rs"));
        let raw = POLICY.map_err(str::to_owned)?;
        let document: SelectionDocument = toml::from_str(raw)
            .map_err(|error| format!("invalid config export policy: {error}"))?;
        let whitelist = Self::validate(template, document.whitelist, "whitelist")?;
        let blacklist = Self::validate(template, document.blacklist, "blacklist")?;
        let paths = match document.default {
            SelectionMode::Whitelist => whitelist,
            SelectionMode::Blacklist => {
                let mut paths = BTreeSet::new();
                Self::collect_leaves(template, "", &mut paths);
                paths.retain(|path| !blacklist.contains(path));
                paths
            }
        };
        Ok(Self {
            paths,
            mode: document.default,
        })
    }

    fn validate(
        template: &Value,
        paths: Vec<String>,
        list: &str,
    ) -> Result<BTreeSet<String>, String> {
        let mut seen = BTreeSet::new();
        for path in paths {
            if path.split('.').any(|part| {
                part.is_empty()
                    || !part
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
            }) {
                return Err(format!("invalid config export {list} path: {path:?}"));
            }
            if seen.contains(&path) {
                return Err(format!("duplicate config export {list} path: {path}"));
            }
            let value = Self::lookup(template, &path).ok_or_else(|| {
                format!("config export {list} path has no configuration template value: {path}")
            })?;
            if value.is_table() {
                return Err(format!(
                    "config export {list} path must select a leaf or a whole array: {path}"
                ));
            }
            seen.insert(path);
        }
        Ok(seen)
    }

    fn collect_leaves(value: &Value, prefix: &str, paths: &mut BTreeSet<String>) {
        if let Some(table) = value.as_table() {
            for (key, value) in table {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                Self::collect_leaves(value, &path, paths);
            }
        } else {
            paths.insert(prefix.to_owned());
        }
    }

    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.paths.iter().map(String::as_str)
    }

    pub fn allows(&self, path: &str) -> bool {
        self.paths.contains(path)
    }

    /// Select fields in document order without normalizing into a full runtime configuration.
    pub fn project(&self, document: &Value) -> Result<Value, String> {
        for path in &self.paths {
            Self::lookup(document, path)
                .ok_or_else(|| format!("selected configuration field is missing: {path}"))?;
        }
        let table = document
            .as_table()
            .ok_or_else(|| "configuration projection requires a table".to_string())?;
        Ok(Value::Table(self.project_table(table, "")))
    }

    fn project_table(&self, table: &Table, prefix: &str) -> Table {
        let mut selected = Table::new();
        for (key, value) in table {
            let path = if prefix.is_empty() {
                key.to_owned()
            } else {
                format!("{prefix}.{key}")
            };
            if let Some(table) = value.as_table() {
                let projected = self.project_table(table, &path);
                if matches!(self.mode, SelectionMode::Blacklist) || !projected.is_empty() {
                    selected.insert(key.clone(), Value::Table(projected));
                }
            } else if self.allows(&path) {
                selected.insert(key.clone(), value.clone());
            }
        }
        selected
    }

    fn lookup<'a>(document: &'a Value, path: &str) -> Option<&'a Value> {
        path.split('.')
            .try_fold(document, |value, part| value.as_table()?.get(part))
    }
}
