//! Describe launch executables in the caller's filesystem.

use std::ffi::{CString, OsStr, OsString};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use tls_probe_point_finder::EntryElfResolver;

use control_contract::command::LaunchTarget;

pub(super) struct LaunchTargetResolver;

impl LaunchTargetResolver {
    pub fn resolve(
        command: &[OsString],
        agent_commands: &[String],
    ) -> Result<LaunchTarget, String> {
        let program = command
            .first()
            .ok_or_else(|| "launch requires a command after --".to_string())?;
        let executable = Self::executable(program)?;
        let mut candidates = Vec::new();
        Self::add_runtime_candidate(&executable, &mut candidates);
        for candidate in agent_commands {
            if let Ok(path) = Self::executable(OsStr::new(candidate)) {
                if path != executable && !candidates.contains(&path) {
                    Self::add_runtime_candidate(&path, &mut candidates);
                    candidates.push(path);
                }
            }
        }
        Ok(LaunchTarget {
            executable,
            candidates,
        })
    }

    fn executable(program: &OsStr) -> Result<PathBuf, String> {
        if program.as_bytes().contains(&b'/') {
            return std::fs::canonicalize(program).map_err(|error| {
                format!("resolve executable {}: {error}", program.to_string_lossy())
            });
        }
        for directory in std::env::split_paths(&Self::search_path()?) {
            let candidate = directory.join(program);
            let Ok(raw) = CString::new(candidate.as_os_str().as_bytes()) else {
                continue;
            };
            if candidate.is_file() && unsafe { libc::access(raw.as_ptr(), libc::X_OK) } == 0 {
                return std::fs::canonicalize(&candidate)
                    .map_err(|error| format!("resolve {}: {error}", candidate.display()));
            }
        }
        Err(format!(
            "executable not found on PATH: {}",
            program.to_string_lossy()
        ))
    }

    fn search_path() -> Result<OsString, String> {
        if let Some(path) = std::env::var_os("PATH") {
            return Ok(path);
        }
        let size = unsafe { libc::confstr(libc::_CS_PATH, std::ptr::null_mut(), 0) };
        if size == 0 {
            return Err("system executable search path is unavailable".to_string());
        }
        let mut path = vec![0; size];
        let written = unsafe { libc::confstr(libc::_CS_PATH, path.as_mut_ptr().cast(), size) };
        if written == 0 || written > size {
            return Err("cannot read system executable search path".to_string());
        }
        path.truncate(written - 1);
        Ok(OsString::from_vec(path))
    }

    fn add_runtime_candidate(path: &Path, candidates: &mut Vec<PathBuf>) {
        if let Ok(runtime) = EntryElfResolver::resolve(path) {
            if runtime != path && !candidates.contains(&runtime) {
                candidates.push(runtime);
            }
        }
    }
}
