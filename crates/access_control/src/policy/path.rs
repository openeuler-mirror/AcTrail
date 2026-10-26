use std::collections::BTreeSet;
use std::ffi::OsString;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

/// Resolves a publication-time target, including links to not-yet-created paths.
pub(super) struct PolicyPath {
    remaining_steps: usize,
}

enum PathStep {
    Root,
    Parent,
    Name(OsString),
    LeaveLink(PathBuf),
}

impl PolicyPath {
    const MAX_STEPS_ENV: &str = "ACTRAIL_POLICY_PATH_MAX_STEPS";

    pub(super) fn bind(path: &Path) -> Result<PathBuf, String> {
        if !path.is_absolute() {
            return Err(format!("policy path {} must be absolute", path.display()));
        }
        let mut resolver = Self::from_environment()?;
        let mut pending = Vec::new();
        resolver.push_steps(path, &mut pending)?;
        let mut links = BTreeSet::new();
        let mut resolved = PathBuf::from("/");
        while let Some(step) = pending.pop() {
            match step {
                PathStep::Root => resolved = PathBuf::from("/"),
                PathStep::Parent => {
                    resolved.pop();
                }
                PathStep::LeaveLink(path) => {
                    links.remove(&path);
                }
                PathStep::Name(name) => {
                    let candidate = resolved.join(name);
                    match std::fs::symlink_metadata(&candidate) {
                        Ok(metadata) if metadata.is_symlink() => {
                            if !links.insert(candidate.clone()) {
                                return Err(format!(
                                    "policy path contains a symlink cycle at {}",
                                    candidate.display()
                                ));
                            }
                            resolver.reserve_step()?;
                            let target = std::fs::read_link(&candidate).map_err(|error| {
                                format!("read policy link {}: {error}", candidate.display())
                            })?;
                            pending.push(PathStep::LeaveLink(candidate));
                            resolver.push_steps(&target, &mut pending)?;
                        }
                        Ok(_) => resolved = candidate,
                        Err(error)
                            if matches!(
                                error.kind(),
                                ErrorKind::NotFound | ErrorKind::NotADirectory
                            ) =>
                        {
                            resolved = candidate;
                        }
                        Err(error) => {
                            return Err(format!(
                                "resolve policy path {}: {error}",
                                candidate.display()
                            ));
                        }
                    }
                }
            }
        }
        Ok(resolved)
    }

    fn from_environment() -> Result<Self, String> {
        let remaining_steps = match std::env::var(Self::MAX_STEPS_ENV) {
            Ok(value) => value
                .parse::<usize>()
                .ok()
                .filter(|value| *value > 0)
                .ok_or_else(|| format!("{} must be a positive integer", Self::MAX_STEPS_ENV))?,
            Err(std::env::VarError::NotPresent) => 4096,
            Err(error) => return Err(format!("read {}: {error}", Self::MAX_STEPS_ENV)),
        };
        Ok(Self { remaining_steps })
    }

    /// Reserve before allocating queued work; completed steps never refund budget.
    fn reserve_step(&mut self) -> Result<(), String> {
        self.remaining_steps = self
            .remaining_steps
            .checked_sub(1)
            .ok_or_else(|| format!("policy path resolution exceeds {}", Self::MAX_STEPS_ENV))?;
        Ok(())
    }

    fn push_steps(&mut self, path: &Path, pending: &mut Vec<PathStep>) -> Result<(), String> {
        for component in path.components().rev() {
            if component == Component::CurDir {
                continue;
            }
            self.reserve_step()?;
            pending.push(match component {
                Component::RootDir => PathStep::Root,
                Component::CurDir => continue,
                Component::ParentDir => PathStep::Parent,
                Component::Normal(name) => PathStep::Name(name.to_owned()),
                Component::Prefix(_) => return Err("policy path must be a Unix path".to_string()),
            });
        }
        Ok(())
    }
}
