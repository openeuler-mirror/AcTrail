use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum PathScope {
    Exact(PathBuf),
    Recursive(PathBuf),
}

impl PathScope {
    pub(super) fn matches_path(&self, path: &Path) -> bool {
        match self {
            Self::Exact(exact) => exact == path,
            Self::Recursive(base) => path.starts_with(base),
        }
    }

    pub(super) fn contains_scope(&self, other: &Self) -> bool {
        match self {
            Self::Exact(path) => matches!(other, Self::Exact(other_path) if path == other_path),
            Self::Recursive(base) => match other {
                Self::Exact(path) | Self::Recursive(path) => path.starts_with(base),
            },
        }
    }

    pub(super) fn display_base(&self) -> PathBuf {
        match self {
            Self::Exact(path) | Self::Recursive(path) => path.clone(),
        }
    }

    pub(super) fn is_recursive(&self) -> bool {
        matches!(self, Self::Recursive(_))
    }

    pub(super) fn display_path(&self) -> String {
        match self {
            Self::Exact(path) => path.display().to_string(),
            Self::Recursive(path) => format!("{}/**", path.display()),
        }
    }

    pub(super) fn collect_mark_directories(
        &self,
        directories: &mut BTreeSet<PathBuf>,
    ) -> Result<(), String> {
        match self {
            Self::Exact(path) => {
                let parent = path.parent().ok_or_else(|| {
                    format!("enforcement rule {} has no parent path", path.display())
                })?;
                let metadata = fs::metadata(parent).map_err(|error| {
                    format!(
                        "fanotify open coverage requires existing parent directory {} for {}: {error}",
                        parent.display(),
                        path.display()
                    )
                })?;
                if !metadata.is_dir() {
                    return Err(format!(
                        "fanotify open coverage parent {} for {} is not a directory",
                        parent.display(),
                        path.display()
                    ));
                }
                directories.insert(parent.to_path_buf());
            }
            Self::Recursive(base) => collect_existing_directories(base, directories)?,
        }
        Ok(())
    }
}

pub(super) fn normalized_scope(path: &str) -> Result<PathScope, String> {
    if let Some(base) = path.strip_suffix("/**") {
        if base.is_empty() {
            return Err("recursive path scope base must not be empty".to_string());
        }
        return Ok(PathScope::Recursive(normalized_absolute_path(base)?));
    }
    Ok(PathScope::Exact(normalized_absolute_path(path)?))
}

pub(super) fn absolute_scope(path: &str) -> Result<PathScope, String> {
    if let Some(base) = path.strip_suffix("/**") {
        if base.is_empty() {
            return Err("recursive path scope base must not be empty".to_string());
        }
        return Ok(PathScope::Recursive(absolute_path(base)?));
    }
    Ok(PathScope::Exact(absolute_path(path)?))
}

pub(super) fn canonical_exact_path(path: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(path);
    if !path.is_absolute() {
        return Err(format!("path {} must be absolute", path.display()));
    }
    fs::canonicalize(&path).map_err(|error| format!("canonicalize {}: {error}", path.display()))
}

fn normalized_absolute_path(path: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(path);
    if !path.is_absolute() {
        return Err(format!("path {} must be absolute", path.display()));
    }
    let mut normalized = PathBuf::from("/");
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(part) => normalized.push(part),
            Component::Prefix(_) => {
                return Err(format!(
                    "path {} must be a Unix absolute path",
                    path.display()
                ));
            }
        }
    }
    Ok(normalized)
}

fn absolute_path(path: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(path);
    if !path.is_absolute() {
        return Err(format!("path {} must be absolute", path.display()));
    }
    Ok(path)
}

fn collect_existing_directories(
    root: &Path,
    directories: &mut BTreeSet<PathBuf>,
) -> Result<(), String> {
    directories.insert(root.to_path_buf());
    for entry in fs::read_dir(root).map_err(|error| {
        format!(
            "fanotify recursive open coverage requires existing directory {}: {error}",
            root.display()
        )
    })? {
        let entry = entry.map_err(|error| format!("read_dir {}: {error}", root.display()))?;
        let path = entry.path();
        if path.is_dir() {
            collect_existing_directories(&path, directories)?;
        }
    }
    Ok(())
}
