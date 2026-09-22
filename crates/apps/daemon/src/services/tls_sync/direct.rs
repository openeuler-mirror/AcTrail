//! File-bound asynchronous direct TLS discovery.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex, mpsc};

use collector_event::ExecFileIdentity;
use control_contract::reply::{ControlError, LaunchTlsPlanUnavailableReason};
use ebpf_collector::loader::DynamicTlsProbePlan;
use tls_probe_point_finder::fast::{
    ArchFilter, FastProbeRequest, ProbeConsumer, ProviderFilter, SourceFilter,
};
use tls_probe_point_finder::{
    BinaryAnalysisCache, BinaryIdentity, DirectSharedObjectResolver, ResolveMode,
    resolve_plans_with_analysis_cache,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum DirectSource {
    Executable,
    SharedLibrary,
}

pub(in crate::services) struct DirectObject {
    file: File,
    identity: ExecFileIdentity,
    source: DirectSource,
}

impl DirectObject {
    pub(super) fn open_startup(path: &Path) -> Option<Self> {
        let mut file = File::open(path).ok()?;
        let mut magic = [0; 4];
        file.read_exact(&mut magic).ok()?;
        if &magic != b"\x7fELF" {
            return None;
        }
        let meta = file.metadata().ok()?;
        Some(Self {
            file,
            source: DirectSource::Executable,
            identity: ExecFileIdentity {
                device_major: libc::major(meta.dev()),
                device_minor: libc::minor(meta.dev()),
                inode: meta.ino(),
                size: meta.size(),
                mtime_seconds: meta.mtime(),
                ctime_seconds: meta.ctime(),
                mtime_nanoseconds: u32::try_from(meta.mtime_nsec()).ok()?,
                ctime_nanoseconds: u32::try_from(meta.ctime_nsec()).ok()?,
            },
        })
    }

    pub(in crate::services) fn open(pid: u32, identity: &ExecFileIdentity) -> Option<Self> {
        let file = File::open(format!("/proc/{pid}/exe")).ok()?;
        let object = Self {
            file,
            identity: identity.clone(),
            source: DirectSource::Executable,
        };
        object.matches().then_some(object)
    }

    pub(in crate::services) fn open_mapping(
        pid: u32,
        start: u64,
        end: u64,
        identity: &ExecFileIdentity,
    ) -> Option<Self> {
        if start >= end {
            return None;
        }
        let file = match File::open(format!("/proc/{pid}/map_files/{start:x}-{end:x}")) {
            Ok(file) => file,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                Self::open_current_mapping(pid, start, end, identity)?
            }
            Err(error) => {
                tracing::debug!(
                    pid,
                    start,
                    end,
                    errno = error.raw_os_error(),
                    "TLS mapped file open failed"
                );
                return None;
            }
        };
        let object = Self {
            file,
            identity: identity.clone(),
            source: DirectSource::SharedLibrary,
        };
        if !object.matches() {
            tracing::debug!(
                pid,
                inode = identity.inode,
                "TLS mapped file identity changed"
            );
            return None;
        }
        Some(object)
    }

    fn open_current_mapping(
        pid: u32,
        start: u64,
        end: u64,
        identity: &ExecFileIdentity,
    ) -> Option<File> {
        // The loader can split the original VMA before userspace handles its event.
        // Resolve only this object's overlapping executable mapping, then fstat it.
        let mut maps = BufReader::new(File::open(format!("/proc/{pid}/maps")).ok()?);
        let mut line = String::new();
        loop {
            line.clear();
            if maps.read_line(&mut line).ok()? == 0 {
                return None;
            }
            let mut fields = line.split_whitespace();
            let range = fields.next()?;
            let (left, right) = range.split_once('-')?;
            let left = u64::from_str_radix(left, 16).ok()?;
            let right = u64::from_str_radix(right, 16).ok()?;
            if left >= end {
                return None;
            }
            if right <= start || !fields.next()?.contains('x') {
                continue;
            }
            let _offset = fields.next()?;
            let (major, minor) = fields.next()?.split_once(':')?;
            let inode = fields.next()?.parse::<u64>().ok()?;
            if inode != identity.inode
                || u32::from_str_radix(major, 16).ok()? != identity.device_major
                || u32::from_str_radix(minor, 16).ok()? != identity.device_minor
            {
                continue;
            }
            let file = File::open(format!("/proc/{pid}/map_files/{left:x}-{right:x}")).ok()?;
            tracing::debug!(pid, inode, "TLS mapped file range resolved");
            return Some(file);
        }
    }

    fn key(&self) -> (DirectSource, ExecFileIdentity) {
        (self.source, self.identity.clone())
    }

    fn path(&self) -> PathBuf {
        PathBuf::from(format!("/proc/self/fd/{}", self.file.as_raw_fd()))
    }

    fn matches(&self) -> bool {
        let Ok(meta) = self.file.metadata() else {
            return false;
        };
        let expected = &self.identity;
        libc::major(meta.dev()) == expected.device_major
            && libc::minor(meta.dev()) == expected.device_minor
            && meta.ino() == expected.inode
            && meta.size() == expected.size
            && meta.mtime() == expected.mtime_seconds
            && meta.mtime_nsec() == i64::from(expected.mtime_nanoseconds)
            && meta.ctime() == expected.ctime_seconds
            && meta.ctime_nsec() == i64::from(expected.ctime_nanoseconds)
    }
}

struct DirectPlanFacts {
    identity: BinaryIdentity,
    provider: String,
    points: String,
}

enum DirectFacts {
    Found(Vec<DirectPlanFacts>),
    Unavailable(LaunchTlsPlanUnavailableReason),
}

enum Entry {
    Pending,
    Ready(Arc<DirectFacts>),
}

type Entries = Arc<Mutex<BTreeMap<(DirectSource, ExecFileIdentity), Entry>>>;

pub(in crate::services) struct DirectResolution {
    object: DirectObject,
    facts: Arc<DirectFacts>,
}

impl DirectResolution {
    pub(in crate::services) fn attach(self, collector: &mut ebpf_collector::EbpfCollector) {
        if !self.object.matches() {
            return;
        }
        let DirectFacts::Found(facts) = self.facts.as_ref() else {
            if let DirectFacts::Unavailable(reason) = self.facts.as_ref() {
                tracing::debug!(
                    reason_code = reason.code(),
                    source = ?self.object.source,
                    "TLS object has no direct plan"
                );
            }
            return;
        };
        let path = self.object.path();
        for fact in facts {
            let plan = DynamicTlsProbePlan {
                target: path.clone(),
                binary: path.clone(),
                target_identity: fact.identity.clone(),
                binary_identity: fact.identity.clone(),
                provider: fact.provider.clone(),
                points: fact.points.clone(),
            };
            if let Err(error) = collector.attach_dynamic_tls_plan(&plan) {
                tracing::debug!(stage = %error.stage, source = ?self.object.source, "TLS object attachment failed");
            } else {
                tracing::debug!(
                    inode = self.object.identity.inode,
                    source = ?self.object.source,
                    "TLS direct object attachment ready"
                );
            }
        }
    }
}

pub(super) struct DirectDiscovery {
    entries: Entries,
    capacity: usize,
    completed: mpsc::Receiver<DirectResolution>,
    wake: UnixStream,
}

pub(super) struct DirectWorker {
    entries: Entries,
    capacity: usize,
    completed: mpsc::SyncSender<DirectResolution>,
    wake: UnixStream,
}

impl DirectDiscovery {
    pub(super) fn new(capacity: usize) -> Result<(Self, DirectWorker), ControlError> {
        let (wake, worker_wake) = UnixStream::pair()
            .map_err(|error| ControlError::new("tls_direct_notify", error.to_string()))?;
        wake.set_nonblocking(true)
            .and_then(|_| worker_wake.set_nonblocking(true))
            .map_err(|error| ControlError::new("tls_direct_notify", error.to_string()))?;
        let (sender, completed) = mpsc::sync_channel(capacity);
        let entries = Arc::new(Mutex::new(BTreeMap::new()));
        Ok((
            Self {
                entries: Arc::clone(&entries),
                capacity,
                completed,
                wake,
            },
            DirectWorker {
                entries,
                capacity,
                completed: sender,
                wake: worker_wake,
            },
        ))
    }

    pub(super) fn poll_fd(&self) -> RawFd {
        self.wake.as_raw_fd()
    }

    pub(super) fn submit(
        &self,
        object: DirectObject,
        send: impl FnOnce(DirectObject) -> bool,
    ) -> Option<DirectResolution> {
        let key = object.key();
        let Ok(mut entries) = self.entries.try_lock() else {
            return None;
        };
        match entries.get(&key) {
            Some(Entry::Pending) => {
                tracing::debug!(inode = key.1.inode, source = ?key.0, "TLS direct object request coalesced");
                return None;
            }
            Some(Entry::Ready(facts)) => {
                tracing::debug!(inode = key.1.inode, source = ?key.0, "TLS direct object cache hit");
                return Some(DirectResolution {
                    object,
                    facts: Arc::clone(facts),
                });
            }
            None => {}
        }
        let evicted = if entries.len() >= self.capacity {
            let candidate = entries
                .iter()
                .find_map(|(key, entry)| matches!(entry, Entry::Ready(_)).then(|| key.clone()));
            let Some(candidate) = candidate else {
                tracing::debug!("TLS direct discovery capacity reached");
                return None;
            };
            entries.remove(&candidate)
        } else {
            None
        };
        entries.insert(key.clone(), Entry::Pending);
        drop(entries);
        drop(evicted);
        if !send(object) {
            if let Ok(mut entries) = self.entries.lock() {
                entries.remove(&key);
            }
        }
        None
    }

    pub(super) fn drain(&mut self) -> Vec<DirectResolution> {
        let mut buffer = [0; 128];
        while matches!(self.wake.read(&mut buffer), Ok(count) if count > 0) {}
        self.completed.try_iter().collect()
    }
}

impl DirectWorker {
    pub(super) fn seed_startup(
        &self,
        object: DirectObject,
        plans: impl IntoIterator<Item = (BinaryIdentity, String, String)>,
    ) {
        let plans = plans
            .into_iter()
            .map(|(identity, provider, points)| DirectPlanFacts {
                identity,
                provider,
                points,
            })
            .collect::<Vec<_>>();
        if !object.matches() {
            return;
        }
        let facts = Arc::new(if plans.is_empty() {
            DirectFacts::Unavailable(LaunchTlsPlanUnavailableReason::NoProbePoints)
        } else {
            DirectFacts::Found(plans)
        });
        let Ok(mut entries) = self.entries.lock() else {
            return;
        };
        if entries.contains_key(&object.key()) {
            return;
        }
        let evicted = if entries.len() >= self.capacity {
            let candidate = entries
                .iter()
                .find_map(|(key, entry)| matches!(entry, Entry::Ready(_)).then(|| key.clone()));
            let Some(candidate) = candidate else {
                return;
            };
            entries.remove(&candidate)
        } else {
            None
        };
        entries.insert(object.key(), Entry::Ready(facts));
        drop(entries);
        drop(evicted);
    }

    pub(super) fn resolve(
        &mut self,
        object: DirectObject,
        cache: &Rc<BinaryAnalysisCache>,
        match_limit: usize,
    ) {
        let path = object.path();
        tracing::debug!(
            inode = object.identity.inode,
            source = ?object.source,
            "TLS direct object analysis started"
        );
        let resolution = match object.source {
            DirectSource::SharedLibrary => {
                DirectSharedObjectResolver::new(Rc::clone(cache)).resolve(&path)
            }
            DirectSource::Executable => resolve_plans_with_analysis_cache(
                FastProbeRequest {
                    binary: path.clone(),
                    arch: ArchFilter::Auto,
                    provider: ProviderFilter::Auto,
                    source: SourceFilter::Executable,
                    match_limit,
                    libraries: Vec::new(),
                    library_search_dirs: Vec::new(),
                },
                ProbeConsumer::Direct,
                ResolveMode::All,
                Rc::clone(cache),
            ),
        };
        let facts = match resolution {
            Ok(resolution) => {
                let mut facts = Vec::new();
                let mut rejected = false;
                for plan in resolution.plans {
                    if plan.binary.path != path || plan.target.binary != path {
                        rejected = true;
                        break;
                    }
                    let Ok(points) = tls_payload_sync::encode_points(&plan) else {
                        rejected = true;
                        break;
                    };
                    facts.push(DirectPlanFacts {
                        identity: plan.binary.identity.clone(),
                        provider: plan.provider.as_str().to_owned(),
                        points,
                    });
                }
                if rejected {
                    DirectFacts::Unavailable(LaunchTlsPlanUnavailableReason::AnalysisRejected)
                } else if facts.is_empty() {
                    DirectFacts::Unavailable(LaunchTlsPlanUnavailableReason::NoProbePoints)
                } else {
                    DirectFacts::Found(facts)
                }
            }
            Err(_) => DirectFacts::Unavailable(LaunchTlsPlanUnavailableReason::AnalysisRejected),
        };
        if !object.matches() {
            if let Ok(mut entries) = self.entries.lock() {
                entries.remove(&object.key());
            }
            return;
        }
        let facts = Arc::new(facts);
        if let Ok(mut entries) = self.entries.lock() {
            entries.insert(object.key(), Entry::Ready(Arc::clone(&facts)));
        }
        if self
            .completed
            .try_send(DirectResolution { object, facts })
            .is_ok()
        {
            loop {
                match self.wake.write(&[1]) {
                    Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                    // A full notification socket is already readable by the daemon.
                    Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                    _ => break,
                }
            }
        } else {
            tracing::debug!("TLS direct discovery completion dropped");
        }
    }
}
