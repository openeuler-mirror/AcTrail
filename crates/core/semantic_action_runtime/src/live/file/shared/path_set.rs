use std::collections::BTreeMap;

use model_core::ids::TraceId;
use semantic_action::{
    FilePathSetState, FilePathSetWrite, file_path_set_identity_for_overflow_scope,
    file_path_set_identity_for_paths,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::live::file) struct FileSummaryPathAccumulator {
    max_paths_per_set: u32,
    path_set_chunk_max_paths: u32,
    path_order_by_path: BTreeMap<String, u32>,
    path_overflow: bool,
    error_path_order_by_path: BTreeMap<String, u32>,
    error_path_overflow: bool,
    error_count: u64,
    error_reason_counts: BTreeMap<String, u64>,
}

impl FileSummaryPathAccumulator {
    pub(in crate::live::file) fn new(
        max_paths_per_set: u32,
        path_set_chunk_max_paths: u32,
    ) -> Self {
        Self {
            max_paths_per_set,
            path_set_chunk_max_paths,
            path_order_by_path: BTreeMap::new(),
            path_overflow: false,
            error_path_order_by_path: BTreeMap::new(),
            error_path_overflow: false,
            error_count: 0,
            error_reason_counts: BTreeMap::new(),
        }
    }

    pub(in crate::live::file) fn record_path(&mut self, path: &str) {
        if record_stable_bounded_path(&mut self.path_order_by_path, self.max_paths_per_set, path) {
            self.path_overflow = true;
        }
    }

    pub(in crate::live::file) fn record_error(&mut self, result: i32, path: &str) {
        self.error_count = self.error_count.saturating_add(1);
        let reason = syscall_error_reason(result);
        let count = self.error_reason_counts.entry(reason).or_insert(0);
        *count = count.saturating_add(1);
        if record_stable_bounded_path(
            &mut self.error_path_order_by_path,
            self.max_paths_per_set,
            path,
        ) {
            self.error_path_overflow = true;
        }
    }

    pub(in crate::live::file) fn stored_path_count(&self) -> u64 {
        self.path_order_by_path.len() as u64
    }

    pub(in crate::live::file) fn error_stored_path_count(&self) -> u64 {
        self.error_path_order_by_path.len() as u64
    }

    pub(in crate::live::file) fn error_count(&self) -> u64 {
        self.error_count
    }

    pub(in crate::live::file) fn path_overflow(&self) -> bool {
        self.path_overflow
    }

    pub(in crate::live::file) fn error_path_overflow(&self) -> bool {
        self.error_path_overflow
    }

    pub(in crate::live::file) fn unique_path_count_state(&self) -> &'static str {
        count_state(self.path_overflow)
    }

    pub(in crate::live::file) fn error_unique_path_count_state(&self) -> &'static str {
        count_state(self.error_path_overflow)
    }

    pub(in crate::live::file) fn chunking_scheme(&self) -> String {
        chunking_scheme_for(self.path_set_chunk_max_paths)
    }

    pub(in crate::live::file) fn path_set_state(&self) -> FilePathSetState {
        if self.path_overflow {
            FilePathSetState::Overflow
        } else {
            FilePathSetState::Complete
        }
    }

    pub(in crate::live::file) fn path_set_id(&self, overflow_scope: Option<&str>) -> String {
        if self.path_set_state() == FilePathSetState::Overflow {
            let scope = overflow_scope
                .filter(|path| path.starts_with('/'))
                .or_else(|| self.path_order_by_path.keys().next().map(String::as_str))
                .unwrap_or("unresolved");
            return file_path_set_identity_for_overflow_scope(&self.chunking_scheme(), scope)
                .path_set_id;
        }
        file_path_set_identity_for_paths(
            self.path_set_state(),
            &self.chunking_scheme(),
            self.path_order_by_path.keys().map(String::as_str),
        )
        .path_set_id
    }

    pub(in crate::live::file) fn path_set_write(
        &self,
        trace_id: TraceId,
        action_id: &str,
        overflow_scope: Option<&str>,
    ) -> Vec<FilePathSetWrite> {
        if self.stored_path_count() == 0 {
            return Vec::new();
        }
        vec![FilePathSetWrite {
            trace_id,
            action_id: action_id.to_string(),
            path_set_id: self.path_set_id(overflow_scope),
            state: self.path_set_state(),
            unique_path_count: self.stored_path_count(),
            stored_path_count: self.stored_path_count(),
            chunking_scheme: self.chunking_scheme(),
            chunk_max_paths: self.path_set_chunk_max_paths,
            paths: self.path_order_by_path.keys().cloned().collect(),
        }]
    }

    pub(in crate::live::file) fn error_reason_counts_text(&self) -> Option<String> {
        if self.error_reason_counts.is_empty() {
            return None;
        }
        Some(format_reason_counts(&self.error_reason_counts))
    }
}

pub(in crate::live::file) fn record_stable_bounded_path(
    paths: &mut BTreeMap<String, u32>,
    max_paths: u32,
    path: &str,
) -> bool {
    if !path.starts_with('/') {
        return false;
    }
    if paths.contains_key(path) {
        return false;
    }
    let max_paths = max_paths as usize;
    if paths.len() < max_paths {
        let path_order = paths.len() as u32;
        paths.insert(path.to_string(), path_order);
        return false;
    }
    if max_paths == 0 {
        return true;
    }
    let Some(largest) = paths.keys().next_back().cloned() else {
        return true;
    };
    if path < largest.as_str() {
        paths.remove(&largest);
        paths.insert(path.to_string(), max_paths as u32 - 1);
    }
    true
}

pub(in crate::live::file) fn chunking_scheme_for(chunk_max_paths: u32) -> String {
    format!("path-id-v1:chunk-max={chunk_max_paths}")
}

fn count_state(path_overflow: bool) -> &'static str {
    if path_overflow {
        "lower_bound"
    } else {
        "exact"
    }
}

fn format_reason_counts(reason_counts: &BTreeMap<String, u64>) -> String {
    reason_counts
        .iter()
        .map(|(reason, count)| format!("{reason}={count}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn syscall_error_reason(result: i32) -> String {
    let errno = result.saturating_abs();
    errno_name(errno)
        .map(str::to_string)
        .unwrap_or_else(|| format!("ERRNO_{errno}"))
}

fn errno_name(errno: i32) -> Option<&'static str> {
    match errno {
        value if value == libc::EACCES => Some("EACCES"),
        value if value == libc::EAGAIN => Some("EAGAIN"),
        value if value == libc::EBADF => Some("EBADF"),
        value if value == libc::EEXIST => Some("EEXIST"),
        value if value == libc::EINTR => Some("EINTR"),
        value if value == libc::EINVAL => Some("EINVAL"),
        value if value == libc::EIO => Some("EIO"),
        value if value == libc::EISDIR => Some("EISDIR"),
        value if value == libc::ELOOP => Some("ELOOP"),
        value if value == libc::EMFILE => Some("EMFILE"),
        value if value == libc::ENFILE => Some("ENFILE"),
        value if value == libc::ENOENT => Some("ENOENT"),
        value if value == libc::ENOMEM => Some("ENOMEM"),
        value if value == libc::ENOSPC => Some("ENOSPC"),
        value if value == libc::ENOTDIR => Some("ENOTDIR"),
        value if value == libc::EPERM => Some("EPERM"),
        value if value == libc::EROFS => Some("EROFS"),
        _ => None,
    }
}
