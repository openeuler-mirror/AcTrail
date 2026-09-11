//! Narrow filesystem adapters for cgroup v2 resource scopes.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::CString;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::{FileExt, MetadataExt, OpenOptionsExt};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const REQUIRED_CONTROLLERS: &[&str] = &["cpu", "memory"];
pub const OPTIONAL_CONTROLLERS: &[&str] = &["io", "pids"];
pub const SYSTEMD_DELEGATION_GUIDANCE: &str = "configure the actraild service with Delegate=yes, DelegateSubgroup=daemon, and a writable delegated cgroup v2 root";

#[derive(Debug)]
pub enum CgroupError {
    UnsafePath(String),
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    Malformed {
        file: &'static str,
        detail: String,
    },
    CounterOverflow {
        file: &'static str,
        counter: &'static str,
    },
    Unsupported(String),
}

impl fmt::Display for CgroupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsafePath(detail) => write!(f, "unsafe cgroup path: {detail}"),
            Self::Io {
                operation,
                path,
                source,
            } => write!(f, "cgroup {operation} {}: {source}", path.display()),
            Self::Malformed { file, detail } => write!(f, "malformed {file}: {detail}"),
            Self::CounterOverflow { file, counter } => {
                write!(f, "counter overflow while aggregating {file}:{counter}")
            }
            Self::Unsupported(detail) => write!(f, "unsupported cgroup configuration: {detail}"),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnifiedHierarchy {
    pub mount_point: PathBuf,
    pub mount_root: PathBuf,
    pub process_relative_path: PathBuf,
    pub process_path: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CgroupPreflightReport {
    pub hierarchy: UnifiedHierarchy,
    pub managed_root: PathBuf,
    pub owner_uid: u32,
    pub owner_gid: u32,
    pub available_controllers: BTreeSet<String>,
    pub enabled_controllers: BTreeSet<String>,
    pub enabled_optional_controllers: BTreeSet<String>,
    pub daemon_path: PathBuf,
    pub disposable_probe_succeeded: bool,
    pub guidance: &'static str,
}

impl std::error::Error for CgroupError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceScopePaths {
    pub relative_path: PathBuf,
    pub aggregate: PathBuf,
    pub workload: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedScopeDirectory {
    pub trace_id: u64,
    pub nonce: String,
    pub paths: TraceScopePaths,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ManagedScopeScan {
    pub managed: Vec<ManagedScopeDirectory>,
    pub unknown: Vec<PathBuf>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MemoryEvents {
    pub low: Option<u64>,
    pub high: Option<u64>,
    pub max: Option<u64>,
    pub oom: Option<u64>,
    pub oom_kill: Option<u64>,
    pub oom_group_kill: Option<u64>,
    pub unknown: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CpuStat {
    pub usage_usec: u64,
    pub user_usec: Option<u64>,
    pub system_usec: Option<u64>,
    pub nr_throttled: Option<u64>,
    pub throttled_usec: Option<u64>,
    pub unknown: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct IoStat {
    pub read_bytes: Option<u64>,
    pub write_bytes: Option<u64>,
    pub unknown: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CgroupCounters {
    pub memory_current_bytes: u64,
    pub memory_peak_bytes: Option<u64>,
    pub memory_anon_bytes: Option<u64>,
    pub memory_file_bytes: Option<u64>,
    pub memory_swap_current_bytes: Option<u64>,
    pub memory_events: Option<MemoryEvents>,
    pub memory_events_local: Option<MemoryEvents>,
    pub cpu: CpuStat,
    pub io: Option<IoStat>,
    pub pids_current: Option<u64>,
    pub pids_peak: Option<u64>,
}

/// Read-only cgroup v2 counter access anchored to an open directory.
///
/// Counter files are opened relative to the directory descriptor with
/// `O_RDONLY | O_CLOEXEC | O_NOFOLLOW`. In particular, `memory.peak` is never
/// opened writable (which would make an accidental peak reset possible). Its
/// descriptor is retained after the first counter read so one reader observes
/// a stable kernel peak window for its lifetime.
#[derive(Debug)]
pub struct CgroupV2CounterReader {
    directory: File,
    display_path: PathBuf,
    memory_peak: OptionalCounterDescriptor,
}

#[derive(Debug)]
enum OptionalCounterDescriptor {
    Unopened,
    Missing,
    Open(File),
}

impl CgroupV2CounterReader {
    /// Open a managed cgroup directory without following a final-component
    /// symlink. Runtime-owned cgroups must instead be resolved beneath the
    /// cgroup2 mount before their verified descriptor is handed to this type.
    pub fn open(directory: impl AsRef<Path>) -> Result<Self, CgroupError> {
        let display_path = directory.as_ref().to_path_buf();
        validate_absolute_path(&display_path)?;
        let directory = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(&display_path)
            .map_err(|source| io_error("open counter directory", &display_path, source))?;
        Self::from_open_directory(directory, display_path)
    }

    /// Construct a reader from a directory descriptor already resolved and
    /// verified by its owner.
    pub fn from_open_directory(
        directory: File,
        display_path: impl Into<PathBuf>,
    ) -> Result<Self, CgroupError> {
        let display_path = display_path.into();
        if !directory
            .metadata()
            .map_err(|source| io_error("inspect counter directory", &display_path, source))?
            .is_dir()
        {
            return Err(CgroupError::UnsafePath(format!(
                "{} is not a cgroup directory",
                display_path.display()
            )));
        }
        Ok(Self {
            directory,
            display_path,
            memory_peak: OptionalCounterDescriptor::Unopened,
        })
    }

    pub fn read_populated(&self) -> Result<bool, CgroupError> {
        parse_populated(&self.read_required("cgroup.events", "cgroup.events")?)
    }

    pub fn read_counters(&mut self) -> Result<CgroupCounters, CgroupError> {
        let memory_current_bytes = parse_single_u64(
            &self.read_required("memory.current", "memory.current")?,
            "memory.current",
        )?;
        let memory_peak_bytes = self.read_memory_peak()?;
        let memory_swap_current_bytes = self
            .read_optional("memory.swap.current", "memory.swap.current")?
            .map(|raw| parse_single_u64(&raw, "memory.swap.current"))
            .transpose()?;
        let memory_stat = self
            .read_optional("memory.stat", "memory.stat")?
            .map(|raw| parse_known_values(&raw, "memory.stat", &["anon", "file"]))
            .transpose()?;
        let memory_events = self
            .read_optional("memory.events", "memory.events")?
            .map(|raw| parse_memory_events(&raw, "memory.events"))
            .transpose()?;
        let memory_events_local = self
            .read_optional("memory.events.local", "memory.events.local")?
            .map(|raw| parse_memory_events(&raw, "memory.events.local"))
            .transpose()?;
        let cpu = parse_cpu_stat(&self.read_required("cpu.stat", "cpu.stat")?)?;
        let io = self
            .read_optional("io.stat", "io.stat")?
            .map(|raw| parse_io_stat(&raw))
            .transpose()?;

        Ok(CgroupCounters {
            memory_current_bytes,
            memory_peak_bytes,
            memory_anon_bytes: memory_stat
                .as_ref()
                .and_then(|values| values.known.get("anon").copied()),
            memory_file_bytes: memory_stat
                .as_ref()
                .and_then(|values| values.known.get("file").copied()),
            memory_swap_current_bytes,
            memory_events,
            memory_events_local,
            cpu,
            io,
            pids_current: self
                .read_optional("pids.current", "pids.current")?
                .map(|raw| parse_single_u64(&raw, "pids.current"))
                .transpose()?,
            pids_peak: self
                .read_optional("pids.peak", "pids.peak")?
                .map(|raw| parse_single_u64(&raw, "pids.peak"))
                .transpose()?,
        })
    }

    fn read_required(&self, name: &'static str, file: &'static str) -> Result<String, CgroupError> {
        let descriptor = self.open_counter(name, file)?;
        read_descriptor(&descriptor, &self.display_path.join(name), file)
    }

    fn read_optional(
        &self,
        name: &'static str,
        file: &'static str,
    ) -> Result<Option<String>, CgroupError> {
        let Some(descriptor) = self.open_optional_counter(name, file)? else {
            return Ok(None);
        };
        read_descriptor(&descriptor, &self.display_path.join(name), file).map(Some)
    }

    fn read_memory_peak(&mut self) -> Result<Option<u64>, CgroupError> {
        if matches!(self.memory_peak, OptionalCounterDescriptor::Unopened) {
            self.memory_peak = match self.open_optional_counter("memory.peak", "memory.peak")? {
                Some(descriptor) => OptionalCounterDescriptor::Open(descriptor),
                None => OptionalCounterDescriptor::Missing,
            };
        }
        match &self.memory_peak {
            OptionalCounterDescriptor::Unopened => unreachable!("memory.peak state initialized"),
            OptionalCounterDescriptor::Missing => Ok(None),
            OptionalCounterDescriptor::Open(descriptor) => parse_single_u64(
                &read_descriptor(
                    descriptor,
                    &self.display_path.join("memory.peak"),
                    "memory.peak",
                )?,
                "memory.peak",
            )
            .map(Some),
        }
    }

    fn open_optional_counter(
        &self,
        name: &'static str,
        file: &'static str,
    ) -> Result<Option<File>, CgroupError> {
        match self.open_counter(name, file) {
            Ok(descriptor) => Ok(Some(descriptor)),
            Err(CgroupError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    fn open_counter(&self, name: &'static str, file: &'static str) -> Result<File, CgroupError> {
        let encoded_name = CString::new(name).expect("static cgroup filename contains no NUL");
        let descriptor = unsafe {
            libc::openat(
                self.directory.as_raw_fd(),
                encoded_name.as_ptr(),
                libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        if descriptor < 0 {
            return Err(CgroupError::Io {
                operation: file,
                path: self.display_path.join(name),
                source: io::Error::last_os_error(),
            });
        }
        Ok(unsafe { File::from_raw_fd(descriptor) })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedCgroupV2 {
    root: PathBuf,
}

impl ManagedCgroupV2 {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, CgroupError> {
        let root = root.into();
        validate_managed_root(&root)?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn trace_paths(&self, trace_id: u64, nonce: &str) -> Result<TraceScopePaths, CgroupError> {
        validate_component(nonce)?;
        let name = format!("trace-{trace_id}-{nonce}");
        let relative_path = PathBuf::from("traces").join(&name);
        let aggregate = self.root.join(&relative_path);
        let workload = aggregate.join("workload");
        Ok(TraceScopePaths {
            relative_path,
            aggregate,
            workload,
        })
    }

    pub fn trace_paths_from_relative(
        &self,
        relative_path: &Path,
    ) -> Result<ManagedScopeDirectory, CgroupError> {
        let components = relative_path.components().collect::<Vec<_>>();
        if components.len() != 2
            || components[0].as_os_str() != "traces"
            || !matches!(components[1], Component::Normal(_))
        {
            return Err(CgroupError::UnsafePath(format!(
                "invalid managed relative path {}",
                relative_path.display()
            )));
        }
        let name = components[1]
            .as_os_str()
            .to_str()
            .ok_or_else(|| CgroupError::UnsafePath("non-UTF-8 scope name".to_string()))?;
        let (trace_id, nonce) = parse_managed_scope_name(name)?;
        let paths = self.trace_paths(trace_id, &nonce)?;
        if paths.relative_path != relative_path {
            return Err(CgroupError::UnsafePath(
                "managed relative path is not canonical".to_string(),
            ));
        }
        Ok(ManagedScopeDirectory {
            trace_id,
            nonce,
            paths,
        })
    }

    pub fn scan_managed_scopes(&self, limit: usize) -> Result<ManagedScopeScan, CgroupError> {
        if limit == 0 {
            return Err(CgroupError::Unsupported(
                "managed scope scan limit must be positive".to_string(),
            ));
        }
        let traces = self.root.join("traces");
        self.verify_descendant(&traces)?;
        require_real_directory(&traces)?;
        let mut scan = ManagedScopeScan::default();
        let mut directory_count = 0_usize;
        for entry in fs::read_dir(&traces)
            .map_err(|source| io_error("scan managed scopes", &traces, source))?
        {
            let entry =
                entry.map_err(|source| io_error("read managed scope entry", &traces, source))?;
            let path = entry.path();
            let file_type = entry
                .file_type()
                .map_err(|source| io_error("inspect managed scope entry", &path, source))?;
            if file_type.is_symlink() {
                scan.unknown.push(path);
                continue;
            }
            if !file_type.is_dir() {
                // Kernel-generated cgroup controller files are expected here
                // and are not orphan scope entries.
                continue;
            }
            directory_count = directory_count.saturating_add(1);
            if directory_count > limit {
                return Err(CgroupError::Unsupported(format!(
                    "managed scope scan exceeded configured limit {limit}"
                )));
            }
            let relative = PathBuf::from("traces").join(entry.file_name());
            match self.trace_paths_from_relative(&relative) {
                Ok(scope)
                    if is_real_directory(&scope.paths.workload)?
                        || scope
                            .paths
                            .workload
                            .symlink_metadata()
                            .is_err_and(|error| error.kind() == io::ErrorKind::NotFound) =>
                {
                    scan.managed.push(scope)
                }
                Ok(_) | Err(_) => scan.unknown.push(path),
            }
        }
        scan.managed.sort_by_key(|scope| scope.trace_id);
        scan.unknown.sort();
        Ok(scan)
    }

    pub fn subtree_is_empty(&self, aggregate: &Path) -> Result<bool, CgroupError> {
        self.verify_descendant(aggregate)?;
        subtree_is_empty_recursive(aggregate)
    }

    /// Returns every process currently charged anywhere below a managed aggregate.
    pub fn read_subtree_pids(&self, aggregate: &Path) -> Result<Vec<u32>, CgroupError> {
        self.verify_descendant(aggregate)?;
        let mut pids = BTreeSet::new();
        collect_subtree_pids_recursive(aggregate, &mut pids)?;
        Ok(pids.into_iter().collect())
    }

    pub fn create_managed_hierarchy(&self) -> Result<(), CgroupError> {
        require_real_directory(&self.root)?;
        create_checked_directory(&self.root.join("daemon"))?;
        create_checked_directory(&self.root.join("traces"))?;
        Ok(())
    }

    pub fn inspect_delegation(
        &self,
        hierarchy: UnifiedHierarchy,
    ) -> Result<CgroupPreflightReport, CgroupError> {
        require_real_directory(&self.root)?;
        if !self.root.starts_with(&hierarchy.mount_point) {
            return Err(CgroupError::Unsupported(format!(
                "managed root {} is outside unified mount {}",
                self.root.display(),
                hierarchy.mount_point.display()
            )));
        }
        let metadata = fs::metadata(&self.root)
            .map_err(|source| io_error("inspect delegated root", &self.root, source))?;
        let available_controllers = read_controller_set(&self.root.join("cgroup.controllers"))?;
        let enabled_controllers = read_controller_set(&self.root.join("cgroup.subtree_control"))?;
        let missing = REQUIRED_CONTROLLERS
            .iter()
            .copied()
            .filter(|controller| !available_controllers.contains(*controller))
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            return Err(CgroupError::Unsupported(format!(
                "delegated root is missing required controllers: {} ({SYSTEMD_DELEGATION_GUIDANCE})",
                missing.join(", ")
            )));
        }
        Ok(CgroupPreflightReport {
            hierarchy,
            managed_root: self.root.clone(),
            owner_uid: metadata.uid(),
            owner_gid: metadata.gid(),
            available_controllers,
            enabled_controllers,
            enabled_optional_controllers: BTreeSet::new(),
            daemon_path: self.root.join("daemon"),
            disposable_probe_succeeded: false,
            guidance: SYSTEMD_DELEGATION_GUIDANCE,
        })
    }

    /// Configures the delegated hierarchy and verifies it with a disposable empty scope.
    ///
    /// The caller must invoke this only after selecting cgroup-v2 or auto mode. The
    /// configured root must already be delegated and writable; this method never creates it.
    pub fn prepare_hierarchy(
        &self,
        hierarchy: UnifiedHierarchy,
        daemon_pid: u32,
    ) -> Result<CgroupPreflightReport, CgroupError> {
        let mut report = self.inspect_delegation(hierarchy.clone())?;
        self.create_managed_hierarchy()?;

        let root_processes = read_pid_list(&self.root.join("cgroup.procs"))?;
        if root_processes.iter().any(|pid| *pid != daemon_pid) {
            return Err(CgroupError::Unsupported(format!(
                "delegated root contains processes other than actraild: {root_processes:?}"
            )));
        }
        self.move_pid(&report.daemon_path, daemon_pid)?;
        self.verify_pid_membership(daemon_pid, &report.daemon_path, &hierarchy)?;

        let controllers = REQUIRED_CONTROLLERS
            .iter()
            .copied()
            .chain(
                OPTIONAL_CONTROLLERS
                    .iter()
                    .copied()
                    .filter(|controller| report.available_controllers.contains(*controller)),
            )
            .collect::<Vec<_>>();
        self.enable_controllers_at_root(&controllers)?;
        self.enable_controllers(&self.root.join("traces"), &controllers)?;
        report.enabled_controllers =
            read_controller_set(&self.root.join("cgroup.subtree_control"))?;
        for required in REQUIRED_CONTROLLERS {
            if !report.enabled_controllers.contains(*required) {
                return Err(CgroupError::Unsupported(format!(
                    "required controller {required} did not become enabled"
                )));
            }
        }
        report.enabled_optional_controllers = OPTIONAL_CONTROLLERS
            .iter()
            .filter(|controller| report.enabled_controllers.contains(**controller))
            .map(|controller| (*controller).to_string())
            .collect();
        if !read_pid_list(&self.root.join("cgroup.procs"))?.is_empty() {
            return Err(CgroupError::Unsupported(
                "delegated root still contains processes after moving actraild to its leaf"
                    .to_string(),
            ));
        }

        self.probe_disposable_scope(&controllers)?;
        report.disposable_probe_succeeded = true;
        Ok(report)
    }

    pub fn create_trace_scope(&self, paths: &TraceScopePaths) -> Result<(), CgroupError> {
        self.verify_scope_paths(paths)?;
        create_new_directory(&paths.aggregate)?;
        if let Err(error) = create_new_directory(&paths.workload) {
            let _ = fs::remove_dir(&paths.aggregate);
            return Err(error);
        }
        Ok(())
    }

    pub fn enable_controllers(
        &self,
        directory: &Path,
        controllers: &[&str],
    ) -> Result<(), CgroupError> {
        self.verify_descendant(directory)?;
        let mut unique = BTreeSet::new();
        for controller in controllers {
            validate_component(controller)?;
            unique.insert(*controller);
        }
        let value = unique
            .into_iter()
            .map(|controller| format!("+{controller}"))
            .collect::<Vec<_>>()
            .join(" ");
        write_control_file(&directory.join("cgroup.subtree_control"), value.as_bytes())
    }

    pub fn enable_trace_controllers(
        &self,
        paths: &TraceScopePaths,
        controllers: &[&str],
    ) -> Result<(), CgroupError> {
        self.verify_scope_paths(paths)?;
        self.enable_controllers(&paths.aggregate, controllers)
    }

    pub fn move_pid(&self, workload: &Path, pid: u32) -> Result<(), CgroupError> {
        self.verify_descendant(workload)?;
        write_control_file(&workload.join("cgroup.procs"), pid.to_string().as_bytes())
    }

    pub fn verify_pid_membership(
        &self,
        pid: u32,
        expected: &Path,
        hierarchy: &UnifiedHierarchy,
    ) -> Result<(), CgroupError> {
        self.verify_descendant(expected)?;
        let raw = read_required(
            &PathBuf::from(format!("/proc/{pid}/cgroup")),
            "proc cgroup membership",
        )?;
        let relative = parse_unified_process_cgroup(&raw)?;
        let actual =
            resolve_process_cgroup_path(&hierarchy.mount_point, &hierarchy.mount_root, &relative)?;
        if actual != expected {
            return Err(CgroupError::Unsupported(format!(
                "PID {pid} membership verification expected {}, got {}",
                expected.display(),
                actual.display()
            )));
        }
        Ok(())
    }

    pub fn counter_reader(&self, aggregate: &Path) -> Result<CgroupV2CounterReader, CgroupError> {
        self.verify_descendant(aggregate)?;
        CgroupV2CounterReader::open(aggregate)
    }

    pub fn read_populated(&self, aggregate: &Path) -> Result<bool, CgroupError> {
        self.counter_reader(aggregate)?.read_populated()
    }

    pub fn read_counters(&self, aggregate: &Path) -> Result<CgroupCounters, CgroupError> {
        self.counter_reader(aggregate)?.read_counters()
    }

    pub fn remove_empty_trace_scope(&self, paths: &TraceScopePaths) -> Result<(), CgroupError> {
        self.verify_scope_paths(paths)?;
        remove_cgroup_subtree(&paths.aggregate)
    }

    fn verify_scope_paths(&self, paths: &TraceScopePaths) -> Result<(), CgroupError> {
        self.verify_descendant(&paths.aggregate)?;
        self.verify_descendant(&paths.workload)?;
        if paths.aggregate.parent() != Some(self.root.join("traces").as_path())
            || paths.workload.parent() != Some(paths.aggregate.as_path())
            || paths.workload.file_name().and_then(|name| name.to_str()) != Some("workload")
        {
            return Err(CgroupError::UnsafePath(
                "trace scope does not match the managed hierarchy".to_string(),
            ));
        }
        Ok(())
    }

    fn verify_descendant(&self, path: &Path) -> Result<(), CgroupError> {
        validate_absolute_path(path)?;
        if path == self.root || !path.starts_with(&self.root) {
            return Err(CgroupError::UnsafePath(format!(
                "{} is outside the configured managed root",
                path.display()
            )));
        }
        Ok(())
    }

    fn enable_controllers_at_root(&self, controllers: &[&str]) -> Result<(), CgroupError> {
        let mut unique = BTreeSet::new();
        for controller in controllers {
            validate_component(controller)?;
            unique.insert(*controller);
        }
        let value = unique
            .into_iter()
            .map(|controller| format!("+{controller}"))
            .collect::<Vec<_>>()
            .join(" ");
        write_control_file(&self.root.join("cgroup.subtree_control"), value.as_bytes())
    }

    fn probe_disposable_scope(&self, controllers: &[&str]) -> Result<(), CgroupError> {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| CgroupError::Unsupported(error.to_string()))?
            .as_nanos();
        let paths =
            self.trace_paths(u64::from(std::process::id()), &format!("preflight{nonce}"))?;
        self.create_trace_scope(&paths)?;
        let result = (|| {
            self.enable_trace_controllers(&paths, controllers)?;
            let _ = self.read_counters(&paths.aggregate)?;
            if self.read_populated(&paths.aggregate)? {
                return Err(CgroupError::Unsupported(
                    "disposable preflight cgroup unexpectedly contains processes".to_string(),
                ));
            }
            Ok(())
        })();
        let cleanup = self.remove_empty_trace_scope(&paths);
        result.and(cleanup)
    }
}

pub fn discover_unified_hierarchy() -> Result<UnifiedHierarchy, CgroupError> {
    let mountinfo = read_required(Path::new("/proc/self/mountinfo"), "proc mountinfo")?;
    let process_cgroup = read_required(Path::new("/proc/self/cgroup"), "proc self cgroup")?;
    discover_unified_hierarchy_from(&mountinfo, &process_cgroup)
}

pub fn discover_unified_hierarchy_from(
    mountinfo: &str,
    process_cgroup: &str,
) -> Result<UnifiedHierarchy, CgroupError> {
    let (mount_root, mount_point) = parse_cgroup2_mount(mountinfo)?;
    let process_relative_path = parse_unified_process_cgroup(process_cgroup)?;
    let process_path =
        resolve_process_cgroup_path(&mount_point, &mount_root, &process_relative_path)?;
    Ok(UnifiedHierarchy {
        mount_point,
        mount_root,
        process_relative_path,
        process_path,
    })
}

pub fn parse_cgroup2_mount(mountinfo: &str) -> Result<(PathBuf, PathBuf), CgroupError> {
    let mut found = None;
    for (line_index, line) in mountinfo.lines().enumerate() {
        let Some((prefix, suffix)) = line.split_once(" - ") else {
            continue;
        };
        let mut suffix_fields = suffix.split_whitespace();
        if suffix_fields.next() != Some("cgroup2") {
            continue;
        }
        let prefix_fields = prefix.split_whitespace().collect::<Vec<_>>();
        if prefix_fields.len() < 5 {
            return malformed("mountinfo", line_index, "missing cgroup2 mount fields");
        }
        let mount_root = PathBuf::from(unescape_mountinfo_path(prefix_fields[3])?);
        let mount_point = PathBuf::from(unescape_mountinfo_path(prefix_fields[4])?);
        if found.replace((mount_root, mount_point)).is_some() {
            return Err(CgroupError::Unsupported(
                "multiple cgroup2 mounts are not supported".to_string(),
            ));
        }
    }
    found.ok_or_else(|| CgroupError::Unsupported("cgroup v2 mount not found".to_string()))
}

/// Shared helper staged for the sandbox PR's guest workload identity. The host
/// managed-scope sampler does not need a mount ID.
pub fn parse_cgroup2_mount_id(mountinfo: &str) -> Result<u64, CgroupError> {
    for (line_index, line) in mountinfo.lines().enumerate() {
        let Some((prefix, suffix)) = line.split_once(" - ") else {
            continue;
        };
        let mut suffix_fields = suffix.split_whitespace();
        if suffix_fields.next() != Some("cgroup2") {
            continue;
        }
        let prefix_fields = prefix.split_whitespace().collect::<Vec<_>>();
        if prefix_fields.len() < 5 {
            return malformed("mountinfo", line_index, "missing cgroup2 mount fields");
        }
        return prefix_fields[0]
            .parse::<u64>()
            .map_err(|_| CgroupError::Malformed {
                file: "mountinfo",
                detail: format!("line {}: invalid cgroup2 mount id", line_index + 1),
            });
    }
    Err(CgroupError::Unsupported(
        "cgroup v2 mount not found".to_string(),
    ))
}

pub fn parse_unified_process_cgroup(raw: &str) -> Result<PathBuf, CgroupError> {
    let mut found = None;
    for (line_index, line) in raw.lines().enumerate() {
        let fields = line.splitn(3, ':').collect::<Vec<_>>();
        if fields.len() != 3 {
            return malformed(
                "proc cgroup",
                line_index,
                "expected hierarchy:controllers:path",
            );
        }
        if fields[0] == "0" && fields[1].is_empty() {
            let path = PathBuf::from(fields[2]);
            validate_absolute_path(&path)?;
            if found.replace(path).is_some() {
                return Err(CgroupError::Malformed {
                    file: "proc cgroup",
                    detail: "duplicate unified hierarchy entry".to_string(),
                });
            }
        }
    }
    found.ok_or_else(|| CgroupError::Unsupported("process is not in cgroup v2".to_string()))
}

pub fn resolve_process_cgroup_path(
    mount_point: &Path,
    mount_root: &Path,
    process_relative_path: &Path,
) -> Result<PathBuf, CgroupError> {
    validate_absolute_path(mount_point)?;
    validate_absolute_path(mount_root)?;
    validate_absolute_path(process_relative_path)?;
    let relative = process_relative_path
        .strip_prefix(mount_root)
        .map_err(|_| {
            CgroupError::Unsupported(format!(
                "process cgroup {} is outside mount root {}",
                process_relative_path.display(),
                mount_root.display()
            ))
        })?;
    Ok(mount_point.join(relative))
}

#[derive(Debug)]
struct ParsedValues {
    known: BTreeMap<String, u64>,
    unknown: BTreeMap<String, String>,
}

pub fn parse_memory_events(raw: &str, file: &'static str) -> Result<MemoryEvents, CgroupError> {
    let values = parse_known_values(
        raw,
        file,
        &["low", "high", "max", "oom", "oom_kill", "oom_group_kill"],
    )?;
    Ok(MemoryEvents {
        low: values.known.get("low").copied(),
        high: values.known.get("high").copied(),
        max: values.known.get("max").copied(),
        oom: values.known.get("oom").copied(),
        oom_kill: values.known.get("oom_kill").copied(),
        oom_group_kill: values.known.get("oom_group_kill").copied(),
        unknown: values.unknown,
    })
}

pub fn parse_cpu_stat(raw: &str) -> Result<CpuStat, CgroupError> {
    let values = parse_known_values(
        raw,
        "cpu.stat",
        &[
            "usage_usec",
            "user_usec",
            "system_usec",
            "nr_throttled",
            "throttled_usec",
        ],
    )?;
    let usage_usec =
        values
            .known
            .get("usage_usec")
            .copied()
            .ok_or_else(|| CgroupError::Malformed {
                file: "cpu.stat",
                detail: "missing usage_usec".to_string(),
            })?;
    Ok(CpuStat {
        usage_usec,
        user_usec: values.known.get("user_usec").copied(),
        system_usec: values.known.get("system_usec").copied(),
        nr_throttled: values.known.get("nr_throttled").copied(),
        throttled_usec: values.known.get("throttled_usec").copied(),
        unknown: values.unknown,
    })
}

pub fn parse_io_stat(raw: &str) -> Result<IoStat, CgroupError> {
    let mut read_bytes = None;
    let mut write_bytes = None;
    let mut unknown = BTreeMap::new();
    for (line_index, line) in raw.lines().enumerate() {
        let mut fields = line.split_whitespace();
        let Some(device) = fields.next() else {
            continue;
        };
        if !device.contains(':') {
            return malformed("io.stat", line_index, "missing device major:minor");
        }
        let mut seen = BTreeSet::new();
        for field in fields {
            let (key, value) = field
                .split_once('=')
                .ok_or_else(|| CgroupError::Malformed {
                    file: "io.stat",
                    detail: format!("line {} has invalid counter {field}", line_index + 1),
                })?;
            if !seen.insert(key) && matches!(key, "rbytes" | "wbytes") {
                return malformed("io.stat", line_index, &format!("duplicate {key}"));
            }
            match key {
                "rbytes" => add_counter(&mut read_bytes, value, "io.stat", "rbytes")?,
                "wbytes" => add_counter(&mut write_bytes, value, "io.stat", "wbytes")?,
                _ => {
                    unknown.insert(format!("{device}.{key}"), value.to_string());
                }
            }
        }
    }
    Ok(IoStat {
        read_bytes,
        write_bytes,
        unknown,
    })
}

pub fn parse_populated(raw: &str) -> Result<bool, CgroupError> {
    let values = parse_known_values(raw, "cgroup.events", &["populated"])?;
    match values.known.get("populated") {
        Some(0) => Ok(false),
        Some(1) => Ok(true),
        Some(other) => Err(CgroupError::Malformed {
            file: "cgroup.events",
            detail: format!("populated must be 0 or 1, got {other}"),
        }),
        None => Err(CgroupError::Malformed {
            file: "cgroup.events",
            detail: "missing populated".to_string(),
        }),
    }
}

fn parse_known_values(
    raw: &str,
    file: &'static str,
    known_keys: &[&str],
) -> Result<ParsedValues, CgroupError> {
    let known_keys = known_keys.iter().copied().collect::<BTreeSet<_>>();
    let mut known = BTreeMap::new();
    let mut unknown = BTreeMap::new();
    for (line_index, line) in raw.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let fields = line.split_whitespace().collect::<Vec<_>>();
        if fields.len() != 2 {
            return malformed(file, line_index, "expected key and value");
        }
        let key = fields[0];
        let value = fields[1];
        if known_keys.contains(key) {
            if known.contains_key(key) {
                return malformed(file, line_index, &format!("duplicate {key}"));
            }
            let parsed = value
                .parse::<u64>()
                .map_err(|error| CgroupError::Malformed {
                    file,
                    detail: format!("line {} invalid {key}: {error}", line_index + 1),
                })?;
            known.insert(key.to_string(), parsed);
        } else {
            unknown.insert(key.to_string(), value.to_string());
        }
    }
    Ok(ParsedValues { known, unknown })
}

fn add_counter(
    total: &mut Option<u64>,
    raw: &str,
    file: &'static str,
    counter: &'static str,
) -> Result<(), CgroupError> {
    let value = raw.parse::<u64>().map_err(|error| CgroupError::Malformed {
        file,
        detail: format!("invalid {counter}: {error}"),
    })?;
    *total = Some(
        total
            .unwrap_or_default()
            .checked_add(value)
            .ok_or(CgroupError::CounterOverflow { file, counter })?,
    );
    Ok(())
}

fn malformed<T>(
    file: &'static str,
    zero_based_line: usize,
    detail: &str,
) -> Result<T, CgroupError> {
    Err(CgroupError::Malformed {
        file,
        detail: format!("line {}: {detail}", zero_based_line + 1),
    })
}

fn validate_managed_root(root: &Path) -> Result<(), CgroupError> {
    validate_absolute_path(root)?;
    if root.file_name().is_none() {
        return Err(CgroupError::UnsafePath(
            "managed root cannot be the filesystem root".to_string(),
        ));
    }
    Ok(())
}

fn validate_absolute_path(path: &Path) -> Result<(), CgroupError> {
    if !path.is_absolute() {
        return Err(CgroupError::UnsafePath(format!(
            "{} is not absolute",
            path.display()
        )));
    }
    for component in path.components() {
        if matches!(component, Component::ParentDir | Component::CurDir) {
            return Err(CgroupError::UnsafePath(format!(
                "{} contains traversal components",
                path.display()
            )));
        }
    }
    Ok(())
}

fn validate_component(value: &str) -> Result<(), CgroupError> {
    if value.is_empty()
        || matches!(value, "." | "..")
        || value.contains('/')
        || value.contains('\\')
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(CgroupError::UnsafePath(format!(
            "invalid component {value:?}"
        )));
    }
    Ok(())
}

fn parse_managed_scope_name(name: &str) -> Result<(u64, String), CgroupError> {
    let rest = name.strip_prefix("trace-").ok_or_else(|| {
        CgroupError::UnsafePath(format!("scope name {name:?} lacks trace prefix"))
    })?;
    let (trace_id, nonce) = rest
        .split_once('-')
        .ok_or_else(|| CgroupError::UnsafePath(format!("scope name {name:?} lacks nonce")))?;
    let trace_id = trace_id.parse::<u64>().map_err(|error| {
        CgroupError::UnsafePath(format!("scope name {name:?} has invalid trace id: {error}"))
    })?;
    validate_component(nonce)?;
    Ok((trace_id, nonce.to_string()))
}

fn subtree_is_empty_recursive(directory: &Path) -> Result<bool, CgroupError> {
    if directory
        .symlink_metadata()
        .is_err_and(|error| error.kind() == io::ErrorKind::NotFound)
    {
        return Ok(true);
    }
    require_real_directory(directory)?;
    if !read_pid_list(&directory.join("cgroup.procs"))?.is_empty() {
        return Ok(false);
    }
    for entry in fs::read_dir(directory)
        .map_err(|source| io_error("scan cgroup subtree", directory, source))?
    {
        let entry =
            entry.map_err(|source| io_error("read cgroup subtree entry", directory, source))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|source| io_error("inspect cgroup subtree entry", &path, source))?;
        if file_type.is_symlink() {
            return Err(CgroupError::UnsafePath(format!(
                "managed subtree contains symlink {}",
                path.display()
            )));
        }
        if file_type.is_dir() && !subtree_is_empty_recursive(&path)? {
            return Ok(false);
        }
    }
    Ok(true)
}

// Only rmdir cgroup directories, never unlink controller files or follow links.
// The kernel refuses populated cgroups; a race/partial failure is retried later.
fn remove_cgroup_subtree(directory: &Path) -> Result<(), CgroupError> {
    if directory
        .symlink_metadata()
        .is_err_and(|error| error.kind() == io::ErrorKind::NotFound)
    {
        return Ok(());
    }
    require_real_directory(directory)?;
    for entry in fs::read_dir(directory)
        .map_err(|source| io_error("scan cgroup cleanup", directory, source))?
    {
        let entry =
            entry.map_err(|source| io_error("read cgroup cleanup entry", directory, source))?;
        let path = entry.path();
        let kind = entry
            .file_type()
            .map_err(|source| io_error("inspect cleanup entry", &path, source))?;
        if kind.is_symlink() {
            return Err(CgroupError::UnsafePath(format!(
                "cgroup cleanup contains symlink {}",
                path.display()
            )));
        }
        if kind.is_dir() {
            remove_cgroup_subtree(&path)?;
        }
    }
    match fs::remove_dir(directory) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(io_error("remove cgroup directory", directory, source)),
    }
}

fn collect_subtree_pids_recursive(
    directory: &Path,
    pids: &mut BTreeSet<u32>,
) -> Result<(), CgroupError> {
    pids.extend(read_pid_list(&directory.join("cgroup.procs"))?);
    for entry in fs::read_dir(directory)
        .map_err(|source| io_error("scan cgroup subtree", directory, source))?
    {
        let entry =
            entry.map_err(|source| io_error("read cgroup subtree entry", directory, source))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|source| io_error("inspect cgroup subtree entry", &path, source))?;
        if file_type.is_symlink() {
            return Err(CgroupError::UnsafePath(format!(
                "managed subtree contains symlink {}",
                path.display()
            )));
        }
        if file_type.is_dir() {
            collect_subtree_pids_recursive(&path, pids)?;
        }
    }
    Ok(())
}

fn unescape_mountinfo_path(raw: &str) -> Result<String, CgroupError> {
    let bytes = raw.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'\\' {
            output.push(bytes[index]);
            index += 1;
            continue;
        }
        let end = index.checked_add(4).ok_or_else(|| CgroupError::Malformed {
            file: "mountinfo",
            detail: "escape length overflow".to_string(),
        })?;
        let escape = bytes
            .get(index + 1..end)
            .ok_or_else(|| CgroupError::Malformed {
                file: "mountinfo",
                detail: format!("truncated escape in {raw:?}"),
            })?;
        let escaped = match escape {
            b"040" => b' ',
            b"011" => b'\t',
            b"012" => b'\n',
            b"134" => b'\\',
            _ => {
                return Err(CgroupError::Malformed {
                    file: "mountinfo",
                    detail: format!("unsupported escape in {raw:?}"),
                });
            }
        };
        output.push(escaped);
        index = end;
    }
    String::from_utf8(output).map_err(|error| CgroupError::Malformed {
        file: "mountinfo",
        detail: error.to_string(),
    })
}

fn read_controller_set(path: &Path) -> Result<BTreeSet<String>, CgroupError> {
    let raw = read_required(path, "controller list")?;
    let mut controllers = BTreeSet::new();
    for controller in raw.split_whitespace() {
        validate_component(controller)?;
        if !controllers.insert(controller.to_string()) {
            return Err(CgroupError::Malformed {
                file: "controller list",
                detail: format!("duplicate controller {controller}"),
            });
        }
    }
    Ok(controllers)
}

fn read_pid_list(path: &Path) -> Result<Vec<u32>, CgroupError> {
    let raw = read_required(path, "cgroup.procs")?;
    raw.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            line.trim()
                .parse::<u32>()
                .map_err(|error| CgroupError::Malformed {
                    file: "cgroup.procs",
                    detail: error.to_string(),
                })
        })
        .collect()
}

fn require_real_directory(path: &Path) -> Result<(), CgroupError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => Err(
            CgroupError::UnsafePath(format!("{} is not a real directory", path.display())),
        ),
        Ok(_) => Ok(()),
        Err(source) => Err(io_error("inspect required directory", path, source)),
    }
}

fn is_real_directory(path: &Path) -> Result<bool, CgroupError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(metadata.is_dir() && !metadata.file_type().is_symlink()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(io_error("inspect directory", path, source)),
    }
}

fn create_checked_directory(path: &Path) -> Result<(), CgroupError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => Err(
            CgroupError::UnsafePath(format!("{} is not a real directory", path.display())),
        ),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => create_new_directory(path),
        Err(source) => Err(io_error("inspect", path, source)),
    }
}

fn create_new_directory(path: &Path) -> Result<(), CgroupError> {
    fs::create_dir(path).map_err(|source| io_error("create directory", path, source))
}

fn remove_directory(path: &Path) -> Result<(), CgroupError> {
    fs::remove_dir(path).map_err(|source| io_error("remove directory", path, source))
}

fn write_control_file(path: &Path, value: &[u8]) -> Result<(), CgroupError> {
    let mut file = OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(|source| io_error("open for write", path, source))?;
    file.write_all(value)
        .map_err(|source| io_error("write", path, source))
}

fn read_required(path: &Path, file: &'static str) -> Result<String, CgroupError> {
    fs::read_to_string(path).map_err(|source| CgroupError::Io {
        operation: file,
        path: path.to_path_buf(),
        source,
    })
}

fn read_descriptor(
    descriptor: &File,
    path: &Path,
    file: &'static str,
) -> Result<String, CgroupError> {
    let mut bytes = Vec::new();
    let mut offset = 0_u64;
    loop {
        let mut chunk = [0_u8; 4096];
        let count = descriptor
            .read_at(&mut chunk, offset)
            .map_err(|source| CgroupError::Io {
                operation: file,
                path: path.to_path_buf(),
                source,
            })?;
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..count]);
        offset = offset
            .checked_add(count as u64)
            .ok_or_else(|| CgroupError::Io {
                operation: file,
                path: path.to_path_buf(),
                source: io::Error::new(io::ErrorKind::InvalidData, "counter size overflow"),
            })?;
    }
    String::from_utf8(bytes).map_err(|source| CgroupError::Io {
        operation: file,
        path: path.to_path_buf(),
        source: io::Error::new(io::ErrorKind::InvalidData, source),
    })
}

fn parse_single_u64(raw: &str, file: &'static str) -> Result<u64, CgroupError> {
    raw.trim()
        .parse::<u64>()
        .map_err(|error| CgroupError::Malformed {
            file,
            detail: error.to_string(),
        })
}

fn io_error(operation: &'static str, path: &Path, source: io::Error) -> CgroupError {
    CgroupError::Io {
        operation,
        path: path.to_path_buf(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_paths_reject_traversal_and_outside_scopes() {
        assert!(ManagedCgroupV2::new("relative/root").is_err());
        assert!(ManagedCgroupV2::new("/").is_err());

        let adapter = ManagedCgroupV2::new("/sys/fs/cgroup/actrail").unwrap();
        assert!(adapter.trace_paths(1, "../escape").is_err());
        assert!(adapter.trace_paths(1, "").is_err());
        assert!(
            adapter
                .verify_descendant(Path::new("/sys/fs/cgroup/other"))
                .is_err()
        );
    }

    #[test]
    fn unified_hierarchy_discovery_handles_mount_roots_and_escapes() {
        let hierarchy = discover_unified_hierarchy_from(
            "29 23 0:26 /delegated /sys/fs/cgroup rw,nosuid,nodev - cgroup2 cgroup rw\n",
            "0::/delegated/service.slice/actraild.service\n",
        )
        .unwrap();
        assert_eq!(hierarchy.mount_point, Path::new("/sys/fs/cgroup"));
        assert_eq!(hierarchy.mount_root, Path::new("/delegated"));
        assert_eq!(
            hierarchy.process_path,
            Path::new("/sys/fs/cgroup/service.slice/actraild.service")
        );

        let (root, point) =
            parse_cgroup2_mount("29 23 0:26 / /sys/fs/cgroup\\040v2 rw - cgroup2 cgroup rw\n")
                .unwrap();
        assert_eq!(root, Path::new("/"));
        assert_eq!(point, Path::new("/sys/fs/cgroup v2"));
    }

    #[test]
    fn unified_hierarchy_discovery_rejects_v1_and_outside_mount_root() {
        assert!(
            discover_unified_hierarchy_from(
                "29 23 0:26 / /sys/fs/cgroup rw - cgroup cgroup rw\n",
                "2:cpu:/service\n"
            )
            .is_err()
        );
        assert!(
            resolve_process_cgroup_path(
                Path::new("/sys/fs/cgroup"),
                Path::new("/delegated"),
                Path::new("/other/service")
            )
            .is_err()
        );
    }

    #[test]
    fn key_value_parsers_tolerate_order_and_preserve_unknown_keys() {
        let events = parse_memory_events(
            "oom_kill 4\nfuture_counter 99\nlow 1\noom_group_kill 5\nhigh 2\nmax 3\noom 6\n",
            "memory.events",
        )
        .unwrap();
        assert_eq!(events.low, Some(1));
        assert_eq!(events.oom_kill, Some(4));
        assert_eq!(
            events.unknown.get("future_counter"),
            Some(&"99".to_string())
        );

        let cpu = parse_cpu_stat(
            "nr_throttled 8\nusage_usec 100\nfuture_cpu value\nuser_usec 60\nsystem_usec 40\n",
        )
        .unwrap();
        assert_eq!(cpu.usage_usec, 100);
        assert_eq!(cpu.user_usec, Some(60));
        assert_eq!(cpu.unknown.get("future_cpu"), Some(&"value".to_string()));
    }

    #[test]
    fn known_duplicate_and_invalid_counters_fail() {
        assert!(matches!(
            parse_cpu_stat("usage_usec 1\nusage_usec 2\n"),
            Err(CgroupError::Malformed { .. })
        ));
        assert!(matches!(
            parse_memory_events("oom nope\n", "memory.events"),
            Err(CgroupError::Malformed { .. })
        ));
        assert!(matches!(
            parse_populated("populated 2\n"),
            Err(CgroupError::Malformed { .. })
        ));
    }

    #[test]
    fn io_stat_aggregates_devices_and_diagnoses_overflow() {
        let io = parse_io_stat("8:0 rbytes=10 wbytes=20 rios=1\n8:16 wbytes=5 rbytes=7 future=3\n")
            .unwrap();
        assert_eq!(io.read_bytes, Some(17));
        assert_eq!(io.write_bytes, Some(25));
        assert_eq!(io.unknown.get("8:16.future"), Some(&"3".to_string()));
        assert!(matches!(
            parse_io_stat(&format!("8:0 rbytes={}\n8:1 rbytes=1\n", u64::MAX)),
            Err(CgroupError::CounterOverflow { .. })
        ));
    }

    #[test]
    fn fake_filesystem_scope_reads_required_and_optional_counters() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("actrail");
        fs::create_dir(&root).unwrap();
        let adapter = ManagedCgroupV2::new(&root).unwrap();
        adapter.create_managed_hierarchy().unwrap();
        let paths = adapter.trace_paths(17, "abc123").unwrap();
        adapter.create_trace_scope(&paths).unwrap();

        write(&paths.aggregate, "memory.current", "4096\n");
        write(&paths.aggregate, "memory.peak", "8192\n");
        write(&paths.aggregate, "memory.stat", "file 1024\nanon 3072\n");
        write(
            &paths.aggregate,
            "memory.events",
            "low 0\nhigh 1\nmax 2\noom 3\noom_kill 4\n",
        );
        write(
            &paths.aggregate,
            "cpu.stat",
            "system_usec 40\nusage_usec 100\nuser_usec 60\n",
        );
        write(&paths.aggregate, "io.stat", "8:0 rbytes=12 wbytes=34\n");
        write(&paths.aggregate, "pids.current", "2\n");
        write(&paths.aggregate, "cgroup.events", "populated 0\nfrozen 0\n");

        let counters = adapter.read_counters(&paths.aggregate).unwrap();
        assert_eq!(counters.memory_current_bytes, 4096);
        assert_eq!(counters.memory_peak_bytes, Some(8192));
        assert_eq!(counters.memory_anon_bytes, Some(3072));
        assert_eq!(counters.memory_swap_current_bytes, None);
        assert_eq!(counters.cpu.usage_usec, 100);
        assert_eq!(counters.io.unwrap().write_bytes, Some(34));
        assert_eq!(counters.pids_current, Some(2));
        assert_eq!(counters.pids_peak, None);
        assert!(!adapter.read_populated(&paths.aggregate).unwrap());

        let empty_paths = adapter.trace_paths(18, "cleanup").unwrap();
        adapter.create_trace_scope(&empty_paths).unwrap();
        adapter.remove_empty_trace_scope(&empty_paths).unwrap();
    }

    #[test]
    fn counter_reader_keeps_memory_peak_descriptor_read_only_and_stable() {
        let temp = tempfile::tempdir().unwrap();
        let aggregate = temp.path().join("trace");
        fs::create_dir(&aggregate).unwrap();
        write(&aggregate, "memory.current", "4096\n");
        write(&aggregate, "memory.peak", "8192\n");
        write(&aggregate, "cpu.stat", "usage_usec 100\n");

        let mut reader = CgroupV2CounterReader::open(&aggregate).unwrap();
        assert_eq!(
            reader.read_counters().unwrap().memory_peak_bytes,
            Some(8192)
        );

        let OptionalCounterDescriptor::Open(memory_peak) = &reader.memory_peak else {
            panic!("memory.peak descriptor was not retained");
        };
        let status_flags = unsafe { libc::fcntl(memory_peak.as_raw_fd(), libc::F_GETFL) };
        assert!(status_flags >= 0);
        assert_eq!(status_flags & libc::O_ACCMODE, libc::O_RDONLY);
        let descriptor_flags = unsafe { libc::fcntl(memory_peak.as_raw_fd(), libc::F_GETFD) };
        assert!(descriptor_flags >= 0);
        assert_ne!(descriptor_flags & libc::FD_CLOEXEC, 0);

        fs::rename(
            aggregate.join("memory.peak"),
            aggregate.join("memory.peak.original"),
        )
        .unwrap();
        write(&aggregate, "memory.peak", "16384\n");
        assert_eq!(
            reader.read_counters().unwrap().memory_peak_bytes,
            Some(8192),
            "the reader must retain its original read-only peak descriptor"
        );
    }

    #[test]
    fn counter_reader_rejects_symlinked_directory_and_counter() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let aggregate = temp.path().join("trace");
        fs::create_dir(&aggregate).unwrap();
        write(&aggregate, "memory.current", "4096\n");
        write(&aggregate, "cpu.stat", "usage_usec 100\n");
        let outside_peak = temp.path().join("outside-memory.peak");
        fs::write(&outside_peak, "8192\n").unwrap();
        symlink(&outside_peak, aggregate.join("memory.peak")).unwrap();

        let linked_directory = temp.path().join("linked-trace");
        symlink(&aggregate, &linked_directory).unwrap();
        assert!(CgroupV2CounterReader::open(&linked_directory).is_err());

        let mut reader = CgroupV2CounterReader::open(&aggregate).unwrap();
        assert!(reader.read_counters().is_err());
    }

    #[test]
    fn managed_scope_scan_is_bounded_and_separates_unknown_entries() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("actrail");
        fs::create_dir(&root).unwrap();
        let adapter = ManagedCgroupV2::new(&root).unwrap();
        adapter.create_managed_hierarchy().unwrap();
        let valid = adapter.trace_paths(4, "known-nonce").unwrap();
        adapter.create_trace_scope(&valid).unwrap();
        fs::create_dir(root.join("traces/not-managed")).unwrap();
        fs::write(root.join("traces/cgroup.controllers"), "cpu memory\n").unwrap();

        let scan = adapter.scan_managed_scopes(2).unwrap();
        assert_eq!(scan.managed.len(), 1);
        assert_eq!(scan.managed[0].trace_id, 4);
        assert_eq!(scan.managed[0].nonce, "known-nonce");
        assert_eq!(scan.unknown, vec![root.join("traces/not-managed")]);
        assert!(adapter.scan_managed_scopes(1).is_err());
    }

    #[test]
    fn recursive_empty_check_observes_descendant_processes() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("actrail");
        fs::create_dir(&root).unwrap();
        let adapter = ManagedCgroupV2::new(&root).unwrap();
        adapter.create_managed_hierarchy().unwrap();
        let paths = adapter.trace_paths(5, "emptycheck").unwrap();
        adapter.create_trace_scope(&paths).unwrap();
        write(&paths.aggregate, "cgroup.procs", "");
        write(&paths.workload, "cgroup.procs", "123\n456\n");
        assert!(!adapter.subtree_is_empty(&paths.aggregate).unwrap());
        assert_eq!(
            adapter.read_subtree_pids(&paths.aggregate).unwrap(),
            vec![123, 456]
        );

        write(&paths.workload, "cgroup.procs", "");
        assert!(adapter.subtree_is_empty(&paths.aggregate).unwrap());
    }

    fn write(directory: &Path, name: &str, value: &str) {
        fs::write(directory.join(name), value).unwrap();
    }

    #[test]
    fn cleanup_removes_nested_scopes_and_is_idempotent_after_partial_progress() {
        let temp = tempfile::tempdir().unwrap();
        let adapter = ManagedCgroupV2::new(temp.path().join("cg")).unwrap();
        fs::create_dir(adapter.root()).unwrap();
        adapter.create_managed_hierarchy().unwrap();
        let paths = adapter.trace_paths(1, "nested").unwrap();
        adapter.create_trace_scope(&paths).unwrap();
        fs::create_dir_all(paths.workload.join("child/grandchild")).unwrap();
        fs::create_dir_all(paths.aggregate.join("sibling/leaf")).unwrap();
        // A regular file blocks rmdir in the fixture, simulating a failed
        // parent removal after all child directories have already disappeared.
        fs::write(paths.aggregate.join("blocker"), "").unwrap();
        assert!(adapter.remove_empty_trace_scope(&paths).is_err());
        assert!(!paths.workload.exists());
        let scan = adapter.scan_managed_scopes(10).unwrap();
        assert!(scan.unknown.is_empty());
        assert_eq!(scan.managed[0].paths, paths);
        fs::remove_file(paths.aggregate.join("blocker")).unwrap();
        adapter.remove_empty_trace_scope(&paths).unwrap();
        adapter.remove_empty_trace_scope(&paths).unwrap();
        assert!(!paths.aggregate.exists());
        assert!(adapter.subtree_is_empty(&paths.aggregate).unwrap());
    }

    #[test]
    fn cleanup_never_follows_symlinked_descendants() {
        let temp = tempfile::tempdir().unwrap();
        let adapter = ManagedCgroupV2::new(temp.path().join("cg")).unwrap();
        fs::create_dir(adapter.root()).unwrap();
        adapter.create_managed_hierarchy().unwrap();
        let paths = adapter.trace_paths(1, "link").unwrap();
        adapter.create_trace_scope(&paths).unwrap();
        let outside = temp.path().join("outside");
        fs::create_dir(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, paths.workload.join("link")).unwrap();
        assert!(adapter.remove_empty_trace_scope(&paths).is_err());
        assert!(outside.is_dir());
    }
}
