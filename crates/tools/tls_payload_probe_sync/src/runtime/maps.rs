//! `/proc/self/maps` address resolution.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

pub(in crate::runtime) fn runtime_address(
    binary: &Path,
    file_offset: u64,
) -> Result<usize, String> {
    let maps = std::fs::read_to_string("/proc/self/maps")
        .map_err(|error| format!("read /proc/self/maps: {error}"))?;
    let binary = canonical(binary);
    for line in maps.lines() {
        let Some(entry) = MapEntry::parse(line) else {
            continue;
        };
        if !entry.executable {
            continue;
        }
        if !same_path(&entry.path, &binary) {
            continue;
        }
        if file_offset >= entry.offset && file_offset < entry.offset + entry.size() {
            return usize::try_from(entry.start + (file_offset - entry.offset))
                .map_err(|error| format!("runtime address overflow: {error}"));
        }
    }
    Err(format!(
        "cannot map file offset 0x{file_offset:x} for {}",
        binary.display()
    ))
}

pub(super) fn is_writable_range(address: usize, length: usize) -> bool {
    let Ok(maps) = std::fs::read_to_string("/proc/self/maps") else {
        return false;
    };
    let Ok(start) = u64::try_from(address) else {
        return false;
    };
    let Some(end) = start.checked_add(length as u64) else {
        return false;
    };
    for line in maps.lines() {
        let Some(entry) = MapEntry::parse(line) else {
            continue;
        };
        if entry.writable && start >= entry.start && end <= entry.end {
            return true;
        }
    }
    false
}

pub(super) fn executable_mapped_files() -> Result<Vec<PathBuf>, String> {
    let maps = std::fs::read_to_string("/proc/self/maps")
        .map_err(|error| format!("read /proc/self/maps: {error}"))?;
    let mut files = BTreeSet::new();
    for line in maps.lines() {
        let Some(entry) = MapEntry::parse(line) else {
            continue;
        };
        if entry.executable && entry.path.is_absolute() {
            files.insert(entry.path);
        }
    }
    Ok(files.into_iter().collect())
}

struct MapEntry {
    start: u64,
    end: u64,
    offset: u64,
    executable: bool,
    writable: bool,
    path: PathBuf,
}

impl MapEntry {
    fn parse(line: &str) -> Option<Self> {
        let mut parts = line.split_whitespace();
        let range = parts.next()?;
        let perms = parts.next()?;
        let offset = parts.next()?;
        let _dev = parts.next()?;
        let _inode = parts.next()?;
        let path = parts.next()?;
        let (start, end) = range.split_once('-')?;
        Some(Self {
            start: u64::from_str_radix(start, 16).ok()?,
            end: u64::from_str_radix(end, 16).ok()?,
            offset: u64::from_str_radix(offset, 16).ok()?,
            executable: perms.as_bytes().get(2).is_some_and(|value| *value == b'x'),
            writable: perms.as_bytes().get(1).is_some_and(|value| *value == b'w'),
            path: canonical(Path::new(path)),
        })
    }

    fn size(&self) -> u64 {
        self.end - self.start
    }
}

fn same_path(left: &Path, right: &Path) -> bool {
    left == right || left.as_os_str() == right.as_os_str()
}

fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}
