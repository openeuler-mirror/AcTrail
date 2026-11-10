//! Copies a read-only snapshot back over the live subvolume in place.
//!
//! The live subvolume keeps its identity, so paused processes whose working directory is inside
//! it continue in the restored tree. Connect refuses scopes that hold nested subvolumes; one created
//! later is held by a snapshot only as an empty root-owned directory. Nested subvolumes and mounted
//! directories in the live tree are left as they are.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs::{self, File, FileTimes, FileType};
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::{MetadataExt, chown};
use std::path::{Path, PathBuf};

use checkpoint::BackendError;

use crate::btrfs::{directory_path, is_subvolume_root, run_command};
use crate::{backend_error, io_error};

pub(crate) fn sync_snapshot_into_live(
    copy_path: &Path,
    snapshot: &Path,
    live: &File,
    copy_batch_max_bytes: usize,
) -> Result<(), BackendError> {
    let handle_path = directory_path(live);
    let root_path =
        fs::read_link(&handle_path).map_err(|error| io_error(handle_path.display(), error))?;
    let device = live
        .metadata()
        .map_err(|error| io_error(root_path.display(), error))?
        .dev();
    let live = handle_path.join(".");
    // Mountinfo names the live tree, while every file operation below uses the pinned fd.
    let mounts = mounted_paths()
        .map_err(|error| io_error("/proc/self/mountinfo", error))?
        .into_iter()
        .filter_map(|path| {
            path.strip_prefix(&root_path)
                .ok()
                .map(|path| live.join(path))
        })
        .collect();
    let mut excluded = Vec::new();
    find_preserved_directories(&live, device, &mounts, &mut excluded)?;
    sync_dir(
        copy_path,
        snapshot,
        &live,
        &excluded,
        copy_batch_max_bytes,
    )?;
    restore_metadata_and_links(snapshot, &live, &excluded, &mut BTreeMap::new())
}

/// The mount table includes same-filesystem bind mounts, which device numbers alone miss.
fn mounted_paths() -> std::io::Result<BTreeSet<PathBuf>> {
    let mountinfo = fs::read("/proc/self/mountinfo")?;
    Ok(mountinfo
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| {
            let raw = line
                .split(|byte| *byte == b' ')
                .nth(4)
                .expect("kernel mountinfo contains a mount point");
            let mut path = Vec::with_capacity(raw.len());
            let mut index = 0;
            while index < raw.len() {
                if raw[index] == b'\\' {
                    // mountinfo escapes spaces, tabs, newlines and backslashes in octal.
                    path.push(
                        (raw[index + 1] - b'0') * 64
                            + (raw[index + 2] - b'0') * 8
                            + (raw[index + 3] - b'0'),
                    );
                    index += 4;
                } else {
                    path.push(raw[index]);
                    index += 1;
                }
            }
            PathBuf::from(OsString::from_vec(path))
        })
        .collect())
}

fn find_preserved_directories(
    dir: &Path,
    device: u64,
    mounts: &BTreeSet<PathBuf>,
    found: &mut Vec<PathBuf>,
) -> Result<(), BackendError> {
    for entry in read_dir(dir)? {
        let entry = entry.map_err(|error| io_error(dir.display(), error))?;
        let path = entry.path();
        if mounts.contains(&path) {
            found.push(path);
            continue;
        }
        let metadata = entry
            .metadata()
            .map_err(|error| io_error(entry.path().display(), error))?;
        if metadata.dev() != device {
            found.push(path);
            continue;
        }
        if !metadata.is_dir() {
            continue;
        }
        if is_subvolume_root(&path).map_err(|error| io_error(path.display(), error))? {
            found.push(path);
        } else {
            find_preserved_directories(&path, device, mounts, found)?;
        }
    }
    Ok(())
}

fn sync_dir(
    copy_path: &Path,
    snapshot: &Path,
    live: &Path,
    nested: &[PathBuf],
    copy_batch_max_bytes: usize,
) -> Result<(), BackendError> {
    if !contains_exclude(live, nested) {
        remove_entries_absent_from(snapshot, live)?;
        return copy_into(
            copy_path,
            &[snapshot.join(".")],
            live,
            copy_batch_max_bytes,
        );
    }
    // A preserved subvolume or mount lies below `live`, so walk this level entry by entry.
    for entry in read_dir(live)? {
        let entry = entry.map_err(|error| io_error(live.display(), error))?;
        let live_path = entry.path();
        if nested.contains(&live_path) {
            continue;
        }
        let kind = file_type(&entry)?;
        if !same_kind_exists(&snapshot.join(entry.file_name()), &kind)? {
            if contains_exclude(&live_path, nested) {
                return Err(backend_error(
                    "nested_subvolume_conflict",
                    format!(
                        "cannot remove or replace {} because it contains a live nested subvolume",
                        live_path.display()
                    ),
                ));
            }
            remove_path(&live_path, &kind)?;
        }
    }
    let mut leaves = Vec::new();
    for entry in read_dir(snapshot)? {
        let entry = entry.map_err(|error| io_error(snapshot.display(), error))?;
        let live_path = live.join(entry.file_name());
        if nested.contains(&live_path) {
            continue;
        }
        if file_type(&entry)?.is_dir() {
            if fs::symlink_metadata(&live_path).is_err() {
                fs::create_dir(&live_path).map_err(|error| io_error(live_path.display(), error))?;
            }
            sync_dir(
                copy_path,
                &entry.path(),
                &live_path,
                nested,
                copy_batch_max_bytes,
            )?;
        } else {
            leaves.push(entry.path());
        }
    }
    if leaves.is_empty() {
        return Ok(());
    }
    copy_into(
        copy_path,
        &leaves,
        live,
        copy_batch_max_bytes,
    )
}

fn contains_exclude(dir: &Path, nested: &[PathBuf]) -> bool {
    nested
        .iter()
        .any(|subvolume| subvolume != dir && subvolume.starts_with(dir))
}

/// Restore directory attributes and file links, and remove attributes that archive copying leaves
/// on existing files. Live nested subvolumes and mounts are never overwritten.
fn restore_metadata_and_links(
    snapshot: &Path,
    live: &Path,
    nested: &[PathBuf],
    hard_links: &mut BTreeMap<(u64, u64), PathBuf>,
) -> Result<(), BackendError> {
    for entry in read_dir(snapshot)? {
        let entry = entry.map_err(|error| io_error(snapshot.display(), error))?;
        let live_path = live.join(entry.file_name());
        if nested.contains(&live_path) {
            continue;
        }
        let metadata = entry
            .metadata()
            .map_err(|error| io_error(entry.path().display(), error))?;
        if metadata.is_dir() {
            restore_metadata_and_links(&entry.path(), &live_path, nested, hard_links)?;
        } else {
            restore_xattrs(&entry.path(), &live_path)
                .map_err(|error| io_error(live_path.display(), error))?;
            if metadata.nlink() > 1 {
                let key = (metadata.dev(), metadata.ino());
                if let Some(first) = hard_links.get(&key) {
                    fs::remove_file(&live_path)
                        .and_then(|()| fs::hard_link(first, &live_path))
                        .map_err(|error| io_error(live_path.display(), error))?;
                } else {
                    hard_links.insert(key, live_path);
                }
            }
        }
    }
    let copy = || -> std::io::Result<()> {
        let metadata = fs::metadata(snapshot)?;
        let directory = File::open(live)?;
        chown(live, Some(metadata.uid()), Some(metadata.gid()))?;
        restore_xattrs(snapshot, live)?;
        fs::set_permissions(live, metadata.permissions())?;
        directory.set_times(
            FileTimes::new()
                .set_accessed(metadata.accessed()?)
                .set_modified(metadata.modified()?),
        )
    };
    copy().map_err(|error| io_error(live.display(), error))
}

fn restore_xattrs(snapshot: &Path, live: &Path) -> std::io::Result<()> {
    let source_attributes: Vec<_> = xattr::list(snapshot)?.collect();
    for name in xattr::list(live)? {
        // SELinux labels cannot be removed, even in permissive mode.
        if name != "security.selinux" && !source_attributes.contains(&name) {
            xattr::remove(live, &name)?;
        }
    }
    for name in source_attributes {
        if let Some(value) = xattr::get(snapshot, &name)? {
            xattr::set(live, &name, &value)?;
        }
    }
    Ok(())
}

fn remove_entries_absent_from(snapshot: &Path, live: &Path) -> Result<(), BackendError> {
    for entry in read_dir(live)? {
        let entry = entry.map_err(|error| io_error(live.display(), error))?;
        let live_path = entry.path();
        let kind = file_type(&entry)?;
        let snapshot_path = snapshot.join(entry.file_name());
        if !same_kind_exists(&snapshot_path, &kind)? {
            remove_path(&live_path, &kind)?;
        } else if kind.is_dir() {
            remove_entries_absent_from(&snapshot_path, &live_path)?;
        }
    }
    Ok(())
}

fn copy_into(
    copy_path: &Path,
    mut sources: &[PathBuf],
    destination: &Path,
    copy_batch_max_bytes: usize,
) -> Result<(), BackendError> {
    // Keep each invocation small even when a directory contains tens of thousands of files.
    while !sources.is_empty() {
        let mut bytes = 0;
        let count = sources
            .iter()
            .take_while(|source| {
                bytes += source.as_os_str().len() + 1 + std::mem::size_of::<usize>();
                bytes <= copy_batch_max_bytes
            })
            .count()
            .max(1);
        copy_batch(copy_path, &sources[..count], destination)?;
        sources = &sources[count..];
    }
    Ok(())
}

fn copy_batch(
    copy_path: &Path,
    sources: &[PathBuf],
    destination: &Path,
) -> Result<(), BackendError> {
    let mut args = vec![
        OsString::from("--archive"),
        OsString::from("--force"),
        OsString::from("--reflink=always"),
        OsString::from("--sparse=auto"),
        OsString::from("--"),
    ];
    args.extend(
        sources
            .iter()
            .map(|source| source.as_os_str().to_os_string()),
    );
    args.push(destination.as_os_str().to_os_string());
    run_command(copy_path, &args).map(|_| ())
}

fn same_kind_exists(path: &Path, kind: &FileType) -> Result<bool, BackendError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            let other = metadata.file_type();
            Ok(other.is_dir() == kind.is_dir()
                && other.is_file() == kind.is_file()
                && other.is_symlink() == kind.is_symlink())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(io_error(path.display(), error)),
    }
}

fn remove_path(path: &Path, kind: &FileType) -> Result<(), BackendError> {
    let result = if kind.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    };
    result.map_err(|error| io_error(path.display(), error))
}

fn read_dir(path: &Path) -> Result<fs::ReadDir, BackendError> {
    fs::read_dir(path).map_err(|error| io_error(path.display(), error))
}

fn file_type(entry: &fs::DirEntry) -> Result<FileType, BackendError> {
    entry
        .file_type()
        .map_err(|error| io_error(entry.path().display(), error))
}
