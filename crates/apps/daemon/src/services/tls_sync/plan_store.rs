//! Binary TLS probe-plan storage used by the sync resolver.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

use tls_probe_point_finder::BinaryIdentity;
use tls_probe_point_finder::fast::ProbeConsumer;

/// Cache identity for one probed binary. `path` omits a peer PID for
/// cross-process reuse, while the remaining fields identify the file that was
/// actually reached through the peer's root.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct BinaryPlanKey {
    path: PathBuf,
    device: u64,
    inode: u64,
    len: u64,
    modified: Option<(u64, u32)>,
    identity: BinaryIdentity,
    consumer: ProbeConsumer,
}

#[derive(Clone, Debug)]
pub(super) enum BinaryPlanRecord {
    Found(Vec<BinaryPlanDescriptor>),
    Unsupported(String),
}

#[derive(Clone, Debug)]
pub(super) struct BinaryPlanDescriptor {
    pub(super) binary: PathBuf,
    pub(super) target_identity: BinaryIdentity,
    pub(super) binary_identity: BinaryIdentity,
    pub(super) provider: String,
    pub(super) source: String,
    pub(super) points: String,
}

pub(super) trait BinaryPlanStore {
    fn get(&self, key: &BinaryPlanKey) -> Result<Option<BinaryPlanRecord>, String>;
    fn put(&mut self, key: BinaryPlanKey, record: BinaryPlanRecord) -> Result<(), String>;
}

#[derive(Default)]
pub(super) struct InMemoryBinaryPlanStore {
    records: BTreeMap<BinaryPlanKey, BinaryPlanRecord>,
}

impl BinaryPlanKey {
    pub(super) fn for_path(path: &Path, consumer: ProbeConsumer) -> Result<Self, String> {
        // Inspect the original path: a proc-root path may point into a
        // tracee's mount namespace and not be addressable from the daemon's
        // root. Normalize only the cache key below so different PIDs share a
        // plan entry.
        let metadata = std::fs::metadata(path).map_err(|error| error.to_string())?;
        let identity =
            tls_probe_point_finder::elf_identity(path).map_err(|error| error.to_string())?;
        let (device, inode) = file_ids(&metadata);
        Ok(Self {
            path: cache_path(path),
            device,
            inode,
            len: metadata.len(),
            modified: metadata
                .modified()
                .ok()
                .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
                .map(|duration| (duration.as_secs(), duration.subsec_nanos())),
            identity,
            consumer,
        })
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(unix)]
fn file_ids(metadata: &std::fs::Metadata) -> (u64, u64) {
    use std::os::unix::fs::MetadataExt;
    (metadata.dev(), metadata.ino())
}

#[cfg(not(unix))]
fn file_ids(_metadata: &std::fs::Metadata) -> (u64, u64) {
    (0, 0)
}

impl BinaryPlanStore for InMemoryBinaryPlanStore {
    fn get(&self, key: &BinaryPlanKey) -> Result<Option<BinaryPlanRecord>, String> {
        Ok(self.records.get(key).cloned())
    }

    fn put(&mut self, key: BinaryPlanKey, record: BinaryPlanRecord) -> Result<(), String> {
        self.records.insert(key, record);
        Ok(())
    }
}

fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn cache_path(path: &Path) -> PathBuf {
    if let Some(peer_path) = strip_peer_root_prefix(path) {
        // `/proc/<pid>/root` identifies the tracee, not the binary. Keep the
        // tracee-visible path so all PIDs can share one plan entry.
        peer_path
    } else if is_proc_namespace_path(path) {
        path.to_path_buf()
    } else {
        canonical(path)
    }
}

fn is_proc_namespace_path(path: &Path) -> bool {
    let raw = path.as_os_str().to_string_lossy();
    raw.starts_with("/proc/") && (raw.contains("/root/") || raw.contains("/fd/"))
}

/// Convert `/proc/<pid>/root/<absolute-path>` to `<absolute-path>` for cache
/// keys. Other proc paths, including `/proc/<pid>/fd/N`, retain their full
/// namespace identity because they may refer to different open files.
fn strip_peer_root_prefix(path: &Path) -> Option<PathBuf> {
    let mut components = path.components();
    match (
        components.next()?,
        components.next()?,
        components.next()?,
        components.next()?,
    ) {
        (
            Component::RootDir,
            Component::Normal(first),
            Component::Normal(pid),
            Component::Normal(root),
        ) if first == "proc" && is_pid_component(pid) && root == "root" => {
            let mut peer_path = PathBuf::from("/");
            peer_path.push(components.collect::<PathBuf>());
            Some(peer_path)
        }
        _ => None,
    }
}

fn is_pid_component(component: &std::ffi::OsStr) -> bool {
    let bytes = component.as_encoded_bytes();
    !bytes.is_empty() && bytes.iter().all(|byte| byte.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_pid_from_peer_root_paths() {
        assert_eq!(
            strip_peer_root_prefix(Path::new("/proc/1234/root/usr/lib64/libc.so.6")),
            Some(PathBuf::from("/usr/lib64/libc.so.6"))
        );
        assert_eq!(strip_peer_root_prefix(Path::new("/proc/12/fd/3")), None);
        assert_eq!(
            strip_peer_root_prefix(Path::new("/proc/self/root/usr/lib64/libc.so.6")),
            None
        );
        assert_eq!(
            strip_peer_root_prefix(Path::new("/proc/abc/root/usr/lib64/libc.so.6")),
            None
        );
    }

    #[test]
    fn peer_root_and_host_paths_share_cache_key() {
        let host = Path::new("/actrail-plan-cache-test/usr/lib64/libc.so.6");
        let first_peer = Path::new("/proc/1234/root/actrail-plan-cache-test/usr/lib64/libc.so.6");
        let second_peer = Path::new("/proc/5678/root/actrail-plan-cache-test/usr/lib64/libc.so.6");
        assert_eq!(cache_path(first_peer), cache_path(host));
        assert_eq!(cache_path(first_peer), cache_path(second_peer));
        assert_ne!(
            cache_path(Path::new("/proc/1234/fd/3")),
            cache_path(Path::new("/proc/5678/fd/3"))
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn same_file_through_peer_root_shares_one_key() {
        let exe = std::env::current_exe().expect("current exe");
        let mut probe = Path::new("/proc")
            .join(std::process::id().to_string())
            .join("root");
        probe.push(exe.strip_prefix("/").expect("absolute exe path"));

        let direct = BinaryPlanKey::for_path(&exe, ProbeConsumer::Sync).expect("direct key");
        let via_peer = BinaryPlanKey::for_path(&probe, ProbeConsumer::Sync).expect("peer key");

        assert_eq!(direct, via_peer);
        assert_eq!(via_peer.path(), exe.as_path());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn different_files_never_share_a_key() {
        let exe = std::env::current_exe().expect("current exe");
        let other = Path::new("/bin/sh");
        if !other.exists() {
            return;
        }

        let first = BinaryPlanKey::for_path(&exe, ProbeConsumer::Sync).expect("first key");
        let second = BinaryPlanKey::for_path(other, ProbeConsumer::Sync).expect("second key");
        assert_ne!(first, second);
    }
}
