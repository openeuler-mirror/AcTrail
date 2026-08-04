use std::path::Path;

use super::PathResolution;

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
    path.resolved.as_deref().and_then(absolute_path)
}

pub(super) fn absolute_path(path: &str) -> Option<String> {
    if !Path::new(path).is_absolute() {
        return None;
    }
    Some(lexically_normalize_path(path))
}
