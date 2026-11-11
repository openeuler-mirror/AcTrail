//! Splits nested scopes by the innermost-owner rule and groups them by backend instance.

use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

use crate::coordinator::CoordinatorError;
use crate::model::ScopeConfig;

/// A scope after nesting is resolved; child scope roots are already in `excludes`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EffectiveScope {
    pub id: String,
    pub root: String,
    pub excludes: Vec<String>,
}

/// The scopes one backend instance receives, in the order the configuration first names it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackendGroup {
    pub backend_instance: String,
    pub scopes: Vec<EffectiveScope>,
}

/// Every path belongs to the innermost scope that covers it, so each child scope's root is added
/// to its parent's excludes. Relative excludes are resolved against the scope root.
pub fn plan_scopes(configs: &[ScopeConfig]) -> Result<Vec<BackendGroup>, CoordinatorError> {
    if configs.is_empty() {
        return Err(invalid("the checkpoint configuration has no scopes"));
    }
    let mut ids = BTreeSet::new();
    let mut roots = Vec::with_capacity(configs.len());
    for config in configs {
        if config.id.is_empty() || !ids.insert(config.id.as_str()) {
            return Err(invalid(format!(
                "scope id {:?} is empty or repeated",
                config.id
            )));
        }
        let root = absolute(&config.root, &config.id)?;
        if roots.contains(&root) {
            return Err(invalid(format!(
                "two scopes share the root {}",
                root.display()
            )));
        }
        roots.push(root);
    }

    let mut groups: Vec<BackendGroup> = Vec::new();
    for (config, root) in configs.iter().zip(&roots) {
        let mut excludes = Vec::with_capacity(config.excludes.len());
        for exclude in &config.excludes {
            let path = if Path::new(exclude).is_absolute() {
                absolute(exclude, &config.id)?
            } else {
                absolute(&root.join(exclude).to_string_lossy(), &config.id)?
            };
            if !strictly_inside(&path, root) {
                return Err(invalid(format!(
                    "exclude {} of scope {} is not inside its root",
                    path.display(),
                    config.id
                )));
            }
            excludes.push(path);
        }
        excludes.extend(
            roots
                .iter()
                .filter(|other| strictly_inside(other, root))
                .cloned(),
        );
        let scope = EffectiveScope {
            id: config.id.clone(),
            root: path_string(root),
            excludes: outermost(excludes)
                .iter()
                .map(|path| path_string(path))
                .collect(),
        };
        match groups
            .iter_mut()
            .find(|group| group.backend_instance == config.backend_instance)
        {
            Some(group) => group.scopes.push(scope),
            None => groups.push(BackendGroup {
                backend_instance: config.backend_instance.clone(),
                scopes: vec![scope],
            }),
        }
    }
    Ok(groups)
}

/// Accepts an absolute path without `.` or `..` components and drops any trailing slash.
fn absolute(raw: &str, scope_id: &str) -> Result<PathBuf, CoordinatorError> {
    let path = Path::new(raw);
    let plain = path
        .components()
        .all(|component| matches!(component, Component::RootDir | Component::Normal(_)));
    if !path.is_absolute() || !plain {
        return Err(invalid(format!(
            "path {raw:?} of scope {scope_id} must be absolute without . or .. components"
        )));
    }
    Ok(path.components().collect())
}

fn strictly_inside(path: &Path, root: &Path) -> bool {
    path != root && path.starts_with(root)
}

/// Drops excludes that lie inside another exclude, and repeats.
fn outermost(mut excludes: Vec<PathBuf>) -> Vec<PathBuf> {
    excludes.sort();
    excludes.dedup();
    let all = excludes.clone();
    excludes.retain(|path| !all.iter().any(|other| strictly_inside(path, other)));
    excludes
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn invalid(message: impl Into<String>) -> CoordinatorError {
    CoordinatorError::new("invalid_scope", message)
}
