//! Btrfs checkpoint backend: one read-only snapshot per scope and save, restored in place.

mod btrfs;
mod config;
mod sync;

use std::collections::BTreeMap;
use std::fmt::Display;
use std::fs::{self, DirBuilder};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use abi_stable::export_root_module;
use abi_stable::prefix_type::PrefixTypeTrait;
use abi_stable::sabi_extern_fn;
use abi_stable::sabi_trait::TD_Opaque;
use abi_stable::std_types::{RResult, RVec};
use checkpoint::{
    BackendConfig, BackendError, BackendErrorKind, BackendKind, CheckpointBackend,
    CheckpointModule, CheckpointModuleRef, RestorableCheckpointBackend,
    RestorableCheckpointBackend_TO, RestoreFailure, SnapshotContext, SnapshotId, SnapshotScope,
};

use crate::btrfs::ScopeRoot;
use crate::config::BtrfsConfig;

#[export_root_module]
fn module() -> CheckpointModuleRef {
    CheckpointModule {
        create,
        kind: BackendKind::Filesystem,
    }
    .leak_into_prefix()
}

#[sabi_extern_fn]
fn create(config: BackendConfig) -> RResult<CheckpointBackend, BackendError> {
    BtrfsBackend::new(&config.json)
        .map(|backend| RestorableCheckpointBackend_TO::from_value(backend, TD_Opaque))
        .into()
}

pub struct BtrfsBackend {
    config: BtrfsConfig,
    snapshot_dir: PathBuf,
    lock_dir: PathBuf,
    snapshot_filesystem: String,
    connection: Option<Connection>,
}

/// What this instance protects, set by `connect`.
struct Connection {
    /// Holds this connection's snapshots, one numbered directory per scope.
    directory: PathBuf,
    roots: Vec<ScopeRoot>,
    /// One path per entry of `roots`, for each snapshot that has not been deleted.
    snapshots: BTreeMap<u64, Vec<PathBuf>>,
    next_snapshot: u64,
}

impl BtrfsBackend {
    pub fn new(config: &str) -> Result<Self, BackendError> {
        let config = BtrfsConfig::parse(config)?;
        let snapshot_dir = config.snapshot_root.join("snapshots");
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&snapshot_dir)
            .and_then(|()| fs::set_permissions(&snapshot_dir, fs::Permissions::from_mode(0o700)))
            .map_err(|error| io_error(snapshot_dir.display(), error))?;
        let lock_dir = config.snapshot_root.join("locks");
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&lock_dir)
            .map_err(|error| io_error(lock_dir.display(), error))?;
        let snapshot_filesystem =
            btrfs::filesystem_uuid(&config.findmnt_path, &config.snapshot_root)?;
        Ok(Self {
            config,
            snapshot_dir,
            lock_dir,
            snapshot_filesystem,
            connection: None,
        })
    }

    fn check_scope(&self, scope: &SnapshotScope) -> Result<ScopeRoot, BackendError> {
        let root = canonical_path(&scope.root)?;
        if !self
            .config
            .allowed_workspace_roots
            .iter()
            .any(|allowed| root.starts_with(allowed))
        {
            return Err(invalid_scope(format!(
                "{} is outside allowed_workspace_roots",
                root.display()
            )));
        }
        let pinned = ScopeRoot::open(root.clone())?;
        let access_path = pinned.access_path();
        btrfs::require_btrfs(&access_path)?;
        btrfs::require_subvolume_root(&access_path)?;
        if btrfs::filesystem_uuid(&self.config.findmnt_path, &access_path)?
            != self.snapshot_filesystem
        {
            return Err(invalid_scope(format!(
                "{} is not on the filesystem that holds snapshot_root",
                root.display()
            )));
        }
        // A snapshot leaves nested subvolumes out, so the scope must hold none and nothing can be
        // excluded; each snapshot then covers the whole directory.
        if !scope.excludes.is_empty() {
            return Err(invalid_scope(format!(
                "{} has excludes or nested scopes, which the Btrfs backend does not support",
                root.display()
            )));
        }
        if let Some(tree) = pinned.nested_subvolume()? {
            return Err(invalid_scope(format!(
                "{} contains the nested subvolume with tree ID {tree}, which a Btrfs snapshot \
                 leaves out",
                root.display()
            )));
        }
        Ok(pinned)
    }

    fn bind(&mut self, context: &SnapshotContext) -> Result<(), BackendError> {
        let own_namespace = fs::read_link("/proc/self/ns/mnt")
            .map_err(|error| io_error("/proc/self/ns/mnt", error))?;
        if context.trace_process.mount_namespace.as_str() != own_namespace.to_string_lossy() {
            return Err(backend_error(
                "unsupported_namespace",
                "the Agent must share the backend's mount namespace",
            ));
        }
        let roots = context
            .scopes
            .iter()
            .map(|scope| self.check_scope(scope))
            .collect::<Result<_, _>>()?;
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        self.connection = Some(Connection {
            directory: self
                .snapshot_dir
                .join(format!("{}-{nanos}", context.trace_id)),
            roots,
            snapshots: BTreeMap::new(),
            next_snapshot: 1,
        });
        Ok(())
    }

    fn connection(&mut self) -> Result<&mut Connection, BackendError> {
        self.connection
            .as_mut()
            .ok_or_else(|| backend_error("not_connected", "the backend is not connected"))
    }

    fn save(&mut self) -> Result<SnapshotId, BackendError> {
        let btrfs_path = self.config.btrfs_path.clone();
        let lock_dir = self.lock_dir.clone();
        let connection = self.connection()?;
        let id = connection.next_snapshot;
        connection.next_snapshot += 1;
        let mut paths: Vec<PathBuf> = Vec::with_capacity(connection.roots.len());
        for (index, root) in connection.roots.iter().enumerate() {
            let parent = connection.directory.join(index.to_string());
            let destination = parent.join(id.to_string());
            let snapshot = fs::create_dir_all(&parent)
                .map_err(|error| io_error(parent.display(), error))
                .and_then(|()| {
                    let scope = root.lock(&lock_dir)?;
                    btrfs::create_read_only_snapshot(scope.directory, &destination)
                });
            if let Err(error) = snapshot {
                // The caller gets no ID for these, so they are removed here; a failure only
                // leaves space behind.
                for path in &paths {
                    let _ = btrfs::delete_subvolume(&btrfs_path, path);
                }
                return Err(error);
            }
            paths.push(destination);
        }
        connection.snapshots.insert(id, paths);
        Ok(snapshot_id(id))
    }

    /// Every snapshot stays usable after a restore, so nothing is invalidated.
    fn roll_back(&mut self, id: &SnapshotId) -> Result<(), BackendError> {
        let copy_path = self.config.copy_path.clone();
        let copy_batch_max_bytes = self.config.copy_batch_max_bytes;
        let lock_dir = self.lock_dir.clone();
        let connection = self.connection()?;
        let paths = connection
            .snapshots
            .get(&parse_id(id)?)
            .ok_or_else(|| unknown_snapshot(id))?;
        for (snapshot, root) in paths.iter().zip(&connection.roots) {
            btrfs::require_read_only_snapshot(snapshot)?;
            let scope = root.lock(&lock_dir)?;
            sync::sync_snapshot_into_live(
                &copy_path,
                snapshot,
                scope.directory,
                copy_batch_max_bytes,
            )?;
        }
        Ok(())
    }

    /// A path already removed leaves the record, so another delete retries only the rest.
    fn remove(&mut self, id: &SnapshotId) -> Result<(), BackendError> {
        let btrfs_path = self.config.btrfs_path.clone();
        let connection = self.connection()?;
        let id = parse_id(id)?;
        let Some(paths) = connection.snapshots.get_mut(&id) else {
            return Ok(());
        };
        while let Some(path) = paths.last() {
            btrfs::delete_subvolume(&btrfs_path, path)?;
            paths.pop();
        }
        connection.snapshots.remove(&id);
        Ok(())
    }

    /// Deletes the snapshots still held and their directory. A snapshot that cannot be deleted
    /// does not stop the others; what is left stays recorded for another disconnect.
    fn release(&mut self) -> Result<(), BackendError> {
        let btrfs_path = self.config.btrfs_path.clone();
        let connection = self.connection()?;
        let mut first_failure = None;
        for paths in connection.snapshots.values_mut() {
            paths.retain(|path| match btrfs::delete_subvolume(&btrfs_path, path) {
                Ok(()) => false,
                Err(error) => {
                    first_failure.get_or_insert(error);
                    true
                }
            });
        }
        connection.snapshots.retain(|_, paths| !paths.is_empty());
        if let Some(error) = first_failure {
            return Err(error);
        }
        // Created by the first save, so a connection that never saved has none.
        match fs::remove_dir_all(&connection.directory) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(io_error(connection.directory.display(), error)),
        }
        self.connection = None;
        Ok(())
    }
}

impl RestorableCheckpointBackend for BtrfsBackend {
    fn connect(&mut self, context: &SnapshotContext) -> RResult<(), BackendError> {
        self.bind(context).into()
    }

    fn snapshot(&mut self) -> RResult<SnapshotId, BackendError> {
        self.save().into()
    }

    fn restore(&mut self, snapshot_id: &SnapshotId) -> RResult<RVec<SnapshotId>, RestoreFailure> {
        self.roll_back(snapshot_id)
            .map(|()| RVec::new())
            .map_err(RestoreFailure::from)
            .into()
    }

    fn delete(&mut self, snapshot_id: &SnapshotId) -> RResult<(), BackendError> {
        self.remove(snapshot_id).into()
    }

    fn disconnect(&mut self) -> RResult<(), BackendError> {
        self.release().into()
    }
}

fn snapshot_id(id: u64) -> SnapshotId {
    SnapshotId(id.to_string().into_bytes().into())
}

fn parse_id(id: &SnapshotId) -> Result<u64, BackendError> {
    std::str::from_utf8(&id.0)
        .ok()
        .and_then(|text| text.parse().ok())
        .ok_or_else(|| unknown_snapshot(id))
}

fn unknown_snapshot(id: &SnapshotId) -> BackendError {
    BackendError::new(
        BackendErrorKind::InvalidSnapshot,
        format!("unknown snapshot {}", String::from_utf8_lossy(&id.0)),
    )
}

/// Scope paths must already be canonical so they name the same directory the daemon resolved.
fn canonical_path(raw: &str) -> Result<PathBuf, BackendError> {
    let path = PathBuf::from(raw);
    if !path.is_absolute() {
        return Err(invalid_scope(format!("{raw} is not absolute")));
    }
    let canonical = path
        .canonicalize()
        .map_err(|error| io_error(path.display(), error))?;
    if canonical != path {
        return Err(invalid_scope(format!(
            "{raw} is not canonical; it resolves to {}",
            canonical.display()
        )));
    }
    Ok(canonical)
}

fn invalid_scope(message: impl Into<String>) -> BackendError {
    backend_error("invalid_scope", message)
}

pub(crate) fn backend_error(code: &str, message: impl Into<String>) -> BackendError {
    let kind = if code.starts_with("invalid_") {
        BackendErrorKind::InvalidConfiguration
    } else {
        BackendErrorKind::OperationFailed
    };
    BackendError::new(kind, format!("{code}: {}", message.into()))
}

pub(crate) fn io_error(context: impl Display, error: std::io::Error) -> BackendError {
    backend_error("io", format!("{context}: {error}"))
}
