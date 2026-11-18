use std::path::Path;

use super::PathResolution;

pub(super) fn proc_fd_alias(path: &str) -> Option<(u32, &str)> {
    let tail = path
        .strip_prefix("/proc/self/fd/")
        .or_else(|| path.strip_prefix("/proc/thread-self/fd/"))?;
    let (fd, suffix) = tail.split_once('/').unwrap_or((tail, ""));
    if fd.is_empty() || !fd.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some((fd.parse().ok()?, suffix))
}

pub(super) fn lexically_normalize_path(path: &str) -> String {
    let absolute = path.starts_with('/');
    let mut parts = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if absolute {
                    let _ = parts.pop();
                } else if parts.last().is_some_and(|last| *last != "..") {
                    let _ = parts.pop();
                } else {
                    parts.push(part);
                }
            }
            _ => parts.push(part),
        }
    }
    if absolute {
        if parts.is_empty() {
            return "/".to_string();
        }
        return format!("/{}", parts.join("/"));
    }
    if parts.is_empty() {
        return ".".to_string();
    }
    parts.join("/")
}

pub(super) fn resolved_absolute_path(path: &PathResolution) -> Option<String> {
    // `PathResolution.resolved` is always produced by `lexically_normalize_path`
    // over an absolute base/raw path, so it is already normalized and
    // absolute. Re-normalizing it here is pure redundant work; keep only the
    // defensive absolute check.
    let resolved = path.resolved.as_deref()?;
    Path::new(resolved)
        .is_absolute()
        .then(|| resolved.to_string())
}

pub(super) fn absolute_path(path: &str) -> Option<String> {
    if !Path::new(path).is_absolute() {
        return None;
    }
    Some(lexically_normalize_path(path))
}
