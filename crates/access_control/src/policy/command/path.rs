use std::path::{Component, Path, PathBuf};

pub struct CommandExecutablePath;

impl CommandExecutablePath {
    pub fn normalize_absolute(raw: &str) -> Result<PathBuf, String> {
        if raw.is_empty() {
            return Err("command executable must not be empty".to_string());
        }
        if raw.contains('\0') {
            return Err("command executable contains NUL".to_string());
        }
        let path = Path::new(raw);
        if !path.is_absolute() {
            return Err(format!("command executable {raw} must be absolute"));
        }
        let mut normalized = PathBuf::from("/");
        for component in path.components() {
            match component {
                Component::RootDir | Component::CurDir => {}
                Component::ParentDir => {
                    normalized.pop();
                }
                Component::Normal(part) => normalized.push(part),
                Component::Prefix(_) => {
                    return Err(format!("command executable {raw} must be a Unix path"));
                }
            }
        }
        Ok(normalized)
    }
}
