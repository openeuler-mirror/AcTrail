//! Managed cgroup v1 trace scopes.
//!
//! The v2 layout `<root>/traces/trace-<id>-<nonce>/workload` is mirrored in every
//! selected v1 hierarchy. The root directory is created on demand: v1 has no
//! delegation boundary, so a privileged daemon owns a top-level subtree in each
//! hierarchy, as the Docker cgroupfs driver does.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::CString;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::common::*;

pub const V1_REQUIRED_CONTROLLERS: &[&str] = &["cpuacct", "memory"];
pub const V1_OPTIONAL_CONTROLLERS: &[&str] = &["pids", "blkio"];

/// One mounted v1 hierarchy that carries at least one selected controller.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct V1Hierarchy {
    /// Selected controllers attached to this hierarchy, such as `cpuacct` or `memory`.
    pub controllers: BTreeSet<String>,
    pub mount_point: PathBuf,
    /// The managed root inside this hierarchy, `<mount_point>/<root_name>`.
    pub root: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct V1PreflightReport {
    pub hierarchies: Vec<V1Hierarchy>,
    pub enabled_optional_controllers: BTreeSet<String>,
    pub disposable_probe_succeeded: bool,
}

/// Managed trace scopes replicated across v1 hierarchies.
///
/// `TraceScopePaths` handed out by this type name the primary (`cpuacct`)
/// hierarchy; every other hierarchy uses the same relative path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedCgroupV1 {
    root_name: String,
    hierarchies: Vec<V1Hierarchy>,
}

impl ManagedCgroupV1 {
    pub fn discover(root_name: &str) -> Result<Self, CgroupError> {
        let mountinfo = read_required(Path::new("/proc/self/mountinfo"), "proc mountinfo")?;
        Self::from_mountinfo(&mountinfo, root_name)
    }

    /// Selects one hierarchy per wanted controller from `/proc/self/mountinfo`.
    ///
    /// Only hierarchies mounted at their root are accepted: a namespaced view
    /// cannot place a process outside the subtree it was started in.
    pub fn from_mountinfo(mountinfo: &str, root_name: &str) -> Result<Self, CgroupError> {
        validate_component(root_name)?;
        let wanted = V1_REQUIRED_CONTROLLERS
            .iter()
            .chain(V1_OPTIONAL_CONTROLLERS)
            .copied()
            .collect::<BTreeSet<_>>();
        let mut by_mount = BTreeMap::<PathBuf, BTreeSet<String>>::new();
        let mut seen = BTreeSet::<String>::new();
        for (line_index, line) in mountinfo.lines().enumerate() {
            let Some((prefix, suffix)) = line.split_once(" - ") else {
                continue;
            };
            let suffix_fields = suffix.split_whitespace().collect::<Vec<_>>();
            if suffix_fields.first() != Some(&"cgroup") {
                continue;
            }
            let prefix_fields = prefix.split_whitespace().collect::<Vec<_>>();
            if prefix_fields.len() < 5 || suffix_fields.len() < 3 {
                return malformed("mountinfo", line_index, "truncated cgroup v1 mount");
            }
            if unescape_mountinfo_path(prefix_fields[3])? != "/" {
                continue;
            }
            let mount_point = PathBuf::from(unescape_mountinfo_path(prefix_fields[4])?);
            validate_absolute_path(&mount_point)?;
            for controller in suffix_fields[2].split(',') {
                if wanted.contains(controller) && seen.insert(controller.to_string()) {
                    by_mount
                        .entry(mount_point.clone())
                        .or_default()
                        .insert(controller.to_string());
                }
            }
        }
        let missing = V1_REQUIRED_CONTROLLERS
            .iter()
            .filter(|controller| !seen.contains(**controller))
            .copied()
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            return Err(CgroupError::Unsupported(format!(
                "cgroup v1 hierarchies missing required controllers: {}",
                missing.join(", ")
            )));
        }
        let mut hierarchies = by_mount
            .into_iter()
            .map(|(mount_point, controllers)| V1Hierarchy {
                root: mount_point.join(root_name),
                controllers,
                mount_point,
            })
            .collect::<Vec<_>>();
        // The cpuacct hierarchy is primary: its paths identify the scope.
        hierarchies.sort_by_key(|hierarchy| !hierarchy.controllers.contains("cpuacct"));
        Ok(Self {
            root_name: root_name.to_string(),
            hierarchies,
        })
    }

    pub fn hierarchies(&self) -> &[V1Hierarchy] {
        &self.hierarchies
    }

    pub fn enabled_controllers(&self) -> BTreeSet<String> {
        self.hierarchies
            .iter()
            .flat_map(|hierarchy| hierarchy.controllers.iter().cloned())
            .collect()
    }

    fn primary(&self) -> &V1Hierarchy {
        &self.hierarchies[0]
    }

    /// Creates `<root>/traces` in every hierarchy and proves a disposable scope works.
    pub fn prepare_hierarchy(&self) -> Result<V1PreflightReport, CgroupError> {
        for hierarchy in &self.hierarchies {
            require_real_directory(&hierarchy.mount_point)?;
            create_checked_directory(&hierarchy.root)?;
            create_checked_directory(&hierarchy.root.join("traces"))?;
            if hierarchy.controllers.contains("memory") {
                Self::require_memory_hierarchy(&hierarchy.root)?;
            }
        }
        self.probe_disposable_scope()?;
        Ok(V1PreflightReport {
            hierarchies: self.hierarchies.clone(),
            enabled_optional_controllers: self
                .enabled_controllers()
                .into_iter()
                .filter(|controller| V1_OPTIONAL_CONTROLLERS.contains(&controller.as_str()))
                .collect(),
            disposable_probe_succeeded: true,
        })
    }

    // Without hierarchical accounting, descendant charges would not reach the aggregate.
    fn require_memory_hierarchy(root: &Path) -> Result<(), CgroupError> {
        let path = root.join("memory.use_hierarchy");
        match fs::read_to_string(&path) {
            Ok(raw) if raw.trim() == "1" => Ok(()),
            Ok(_) => Err(CgroupError::Unsupported(format!(
                "{} is disabled",
                path.display()
            ))),
            // Removed by kernels where hierarchical accounting is unconditional.
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(io_error("read memory.use_hierarchy", &path, source)),
        }
    }

    pub fn trace_paths(&self, trace_id: u64, nonce: &str) -> Result<TraceScopePaths, CgroupError> {
        Self::paths_under(&self.primary().root, trace_id, nonce)
    }

    fn paths_under(
        root: &Path,
        trace_id: u64,
        nonce: &str,
    ) -> Result<TraceScopePaths, CgroupError> {
        validate_component(nonce)?;
        let relative_path = PathBuf::from("traces").join(format!("trace-{trace_id}-{nonce}"));
        let aggregate = root.join(&relative_path);
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

    /// The same scope in every hierarchy, primary first.
    fn replicas(&self, paths: &TraceScopePaths) -> Result<Vec<TraceScopePaths>, CgroupError> {
        self.verify_scope_paths(paths)?;
        Ok(self
            .hierarchies
            .iter()
            .map(|hierarchy| {
                let aggregate = hierarchy.root.join(&paths.relative_path);
                TraceScopePaths {
                    relative_path: paths.relative_path.clone(),
                    workload: aggregate.join("workload"),
                    aggregate,
                }
            })
            .collect())
    }

    /// Recovery may accept a scope only when both directories exist in every
    /// selected hierarchy. The scan intentionally finds partial scopes for cleanup.
    pub fn scope_replicas_present(&self, paths: &TraceScopePaths) -> Result<bool, CgroupError> {
        for replica in self.replicas(paths)? {
            if !is_real_directory(&replica.aggregate)? || !is_real_directory(&replica.workload)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn replica_aggregates(&self, aggregate: &Path) -> Result<Vec<PathBuf>, CgroupError> {
        let primary = &self.primary().root;
        validate_absolute_path(aggregate)?;
        let relative = aggregate
            .strip_prefix(primary)
            .ok()
            .filter(|relative| !relative.as_os_str().is_empty())
            .ok_or_else(|| {
                CgroupError::UnsafePath(format!(
                    "{} is outside the configured managed root",
                    aggregate.display()
                ))
            })?;
        Ok(self
            .hierarchies
            .iter()
            .map(|hierarchy| hierarchy.root.join(relative))
            .collect())
    }

    pub fn create_trace_scope(&self, paths: &TraceScopePaths) -> Result<(), CgroupError> {
        let mut created = Vec::new();
        for replica in self.replicas(paths)? {
            if let Err(error) = create_new_directory(&replica.aggregate).and_then(|()| {
                created.push(replica.aggregate.clone());
                create_new_directory(&replica.workload)
            }) {
                for aggregate in created.iter().rev() {
                    let _ = remove_cgroup_subtree(aggregate);
                }
                return Err(error);
            }
        }
        Ok(())
    }

    /// Moves `pid` into the workload leaf of every hierarchy, primary last.
    ///
    /// The primary hierarchy defines scope membership, so it moves only after
    /// every other controller already charges the new scope.
    pub fn move_pid(&self, paths: &TraceScopePaths, pid: u32) -> Result<(), CgroupError> {
        for replica in self.replicas(paths)?.iter().rev() {
            write_control_file(
                &replica.workload.join("cgroup.procs"),
                pid.to_string().as_bytes(),
            )?;
        }
        Ok(())
    }

    pub fn verify_pid_membership(
        &self,
        pid: u32,
        paths: &TraceScopePaths,
    ) -> Result<(), CgroupError> {
        let replicas = self.replicas(paths)?;
        let raw = read_required(
            &PathBuf::from(format!("/proc/{pid}/cgroup")),
            "proc cgroup membership",
        )?;
        for (hierarchy, replica) in self.hierarchies.iter().zip(replicas) {
            let expected = Path::new("/")
                .join(&self.root_name)
                .join(&replica.relative_path)
                .join("workload");
            let actual = parse_v1_process_cgroup(&raw, &hierarchy.controllers)?;
            if actual != expected {
                return Err(CgroupError::Unsupported(format!(
                    "PID {pid} membership verification in {} expected {}, got {}",
                    hierarchy.mount_point.display(),
                    expected.display(),
                    actual.display()
                )));
            }
        }
        Ok(())
    }

    /// v1 has no `cgroup.events`; emptiness is a recursive `cgroup.procs` walk
    /// across every hierarchy.
    pub fn subtree_is_empty(&self, aggregate: &Path) -> Result<bool, CgroupError> {
        for replica in self.replica_aggregates(aggregate)? {
            if !subtree_is_empty_recursive(&replica)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub fn read_populated(&self, aggregate: &Path) -> Result<bool, CgroupError> {
        self.subtree_is_empty(aggregate).map(|empty| !empty)
    }

    pub fn read_subtree_pids(&self, aggregate: &Path) -> Result<Vec<u32>, CgroupError> {
        self.replica_aggregates(aggregate)?;
        read_cgroup_subtree_pids(aggregate)
    }

    pub fn counter_reader(&self, aggregate: &Path) -> Result<CgroupV1CounterReader, CgroupError> {
        let replicas = self.replica_aggregates(aggregate)?;
        let mut reader = CgroupV1CounterReader {
            display_path: aggregate.to_path_buf(),
            cpuacct: None,
            memory: None,
            pids: None,
            blkio: None,
        };
        for (hierarchy, replica) in self.hierarchies.iter().zip(replicas) {
            let directory = open_directory(&replica)?;
            for controller in &hierarchy.controllers {
                let slot = match controller.as_str() {
                    "cpuacct" => &mut reader.cpuacct,
                    "memory" => &mut reader.memory,
                    "pids" => &mut reader.pids,
                    "blkio" => &mut reader.blkio,
                    _ => continue,
                };
                *slot = Some(
                    directory
                        .try_clone()
                        .map_err(|source| io_error("clone counter directory", &replica, source))?,
                );
            }
        }
        if reader.cpuacct.is_none() || reader.memory.is_none() {
            return Err(CgroupError::Unsupported(
                "managed v1 scope lacks cpuacct or memory".to_string(),
            ));
        }
        Ok(reader)
    }

    pub fn read_counters(&self, aggregate: &Path) -> Result<CgroupCounters, CgroupError> {
        self.counter_reader(aggregate)?.read_counters()
    }

    /// Removes the scope from every hierarchy; a failure in one hierarchy does
    /// not stop cleanup of the others, and the first error is reported.
    pub fn remove_empty_trace_scope(&self, paths: &TraceScopePaths) -> Result<(), CgroupError> {
        let mut first_error = None;
        for replica in self.replicas(paths)? {
            if let Err(error) = remove_cgroup_subtree(&replica.aggregate) {
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    /// Scans every hierarchy, so a scope left behind in only one of them is found.
    pub fn scan_managed_scopes(&self, limit: usize) -> Result<ManagedScopeScan, CgroupError> {
        if limit == 0 {
            return Err(CgroupError::Unsupported(
                "managed scope scan limit must be positive".to_string(),
            ));
        }
        let mut managed = BTreeMap::<PathBuf, ManagedScopeDirectory>::new();
        let mut unknown = BTreeSet::new();
        for hierarchy in &self.hierarchies {
            let traces = hierarchy.root.join("traces");
            require_real_directory(&traces)?;
            let mut directory_count = 0_usize;
            for entry in fs::read_dir(&traces)
                .map_err(|source| io_error("scan managed scopes", &traces, source))?
            {
                let entry = entry
                    .map_err(|source| io_error("read managed scope entry", &traces, source))?;
                let path = entry.path();
                let file_type = entry
                    .file_type()
                    .map_err(|source| io_error("inspect managed scope entry", &path, source))?;
                if file_type.is_symlink() {
                    unknown.insert(path);
                    continue;
                }
                if !file_type.is_dir() {
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
                    Ok(scope) => {
                        managed.entry(relative).or_insert(scope);
                    }
                    Err(_) => {
                        unknown.insert(path);
                    }
                }
            }
        }
        let mut scan = ManagedScopeScan {
            managed: managed.into_values().collect(),
            unknown: unknown.into_iter().collect(),
        };
        scan.managed.sort_by_key(|scope| scope.trace_id);
        Ok(scan)
    }

    fn verify_scope_paths(&self, paths: &TraceScopePaths) -> Result<(), CgroupError> {
        let canonical = self.trace_paths_from_relative(&paths.relative_path)?.paths;
        if &canonical != paths {
            return Err(CgroupError::UnsafePath(
                "trace scope does not match the managed hierarchy".to_string(),
            ));
        }
        Ok(())
    }

    fn probe_disposable_scope(&self) -> Result<(), CgroupError> {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| CgroupError::Unsupported(error.to_string()))?
            .as_nanos();
        let paths =
            self.trace_paths(u64::from(std::process::id()), &format!("preflight{nonce}"))?;
        self.create_trace_scope(&paths)?;
        let result = (|| {
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

/// Returns the path of the hierarchy that carries `controllers` in a
/// `/proc/<pid>/cgroup` listing.
pub fn parse_v1_process_cgroup(
    raw: &str,
    controllers: &BTreeSet<String>,
) -> Result<PathBuf, CgroupError> {
    for line in raw.lines() {
        let mut fields = line.splitn(3, ':');
        let (Some(_), Some(listed), Some(path)) = (fields.next(), fields.next(), fields.next())
        else {
            return Err(CgroupError::Malformed {
                file: "proc cgroup membership",
                detail: format!("malformed line {line:?}"),
            });
        };
        let listed = listed.split(',').collect::<BTreeSet<_>>();
        if controllers
            .iter()
            .all(|controller| listed.contains(controller.as_str()))
        {
            return Ok(PathBuf::from(path));
        }
    }
    Err(CgroupError::Unsupported(format!(
        "process has no cgroup v1 membership for {controllers:?}"
    )))
}

fn open_directory(path: &Path) -> Result<File, CgroupError> {
    validate_absolute_path(path)?;
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(path)
        .map_err(|source| io_error("open counter directory", path, source))
}

/// Read-only v1 counters for one managed scope, anchored to open directories.
#[derive(Debug)]
pub struct CgroupV1CounterReader {
    display_path: PathBuf,
    cpuacct: Option<File>,
    memory: Option<File>,
    pids: Option<File>,
    blkio: Option<File>,
}

impl CgroupV1CounterReader {
    pub fn read_subtree_pids(&self) -> Result<Vec<u32>, CgroupError> {
        let directory = self.cpuacct.as_ref().expect("reader requires cpuacct");
        let path = PathBuf::from(format!("/proc/self/fd/{}", directory.as_raw_fd()));
        let mut pids = BTreeSet::new();
        collect_subtree_pids_recursive(&path, &mut pids)?;
        Ok(pids.into_iter().collect())
    }

    /// Maps v1 counters onto the v2 counter model.
    ///
    /// - CPU is `cpuacct.usage` (hierarchical, nanoseconds).
    /// - Memory is `memory.usage_in_bytes` and `max_usage_in_bytes`; `anon`,
    ///   `file` and `swap` come from the hierarchical `total_*` keys of `memory.stat`.
    /// - I/O sums `blkio.throttle.io_service_bytes_recursive` over devices. v1
    ///   charges buffered writeback to the root cgroup, so writes can be undercounted.
    pub fn read_counters(&mut self) -> Result<CgroupCounters, CgroupError> {
        let cpuacct = self.cpuacct.as_ref().expect("reader requires cpuacct");
        let memory = self.memory.as_ref().expect("reader requires memory");
        let usage_ns = parse_single_u64(&self.read(cpuacct, "cpuacct.usage")?, "cpuacct.usage")?;
        let user_ns = self
            .read_optional(cpuacct, "cpuacct.usage_user")?
            .map(|raw| parse_single_u64(&raw, "cpuacct.usage_user"))
            .transpose()?;
        let system_ns = self
            .read_optional(cpuacct, "cpuacct.usage_sys")?
            .map(|raw| parse_single_u64(&raw, "cpuacct.usage_sys"))
            .transpose()?;
        let memory_current_bytes = parse_single_u64(
            &self.read(memory, "memory.usage_in_bytes")?,
            "memory.usage_in_bytes",
        )?;
        let memory_peak_bytes = self
            .read_optional(memory, "memory.max_usage_in_bytes")?
            .map(|raw| parse_single_u64(&raw, "memory.max_usage_in_bytes"))
            .transpose()?;
        let memory_stat = self
            .read_optional(memory, "memory.stat")?
            .map(|raw| {
                parse_known_values(
                    &raw,
                    "memory.stat",
                    &["total_rss", "total_cache", "total_swap"],
                )
            })
            .transpose()?;
        let oom_kill = self
            .read_optional(memory, "memory.oom_control")?
            .map(|raw| parse_known_values(&raw, "memory.oom_control", &["oom_kill"]))
            .transpose()?
            .and_then(|values| values.known.get("oom_kill").copied());
        let stat = |key: &str| {
            memory_stat
                .as_ref()
                .and_then(|values| values.known.get(key).copied())
        };
        let io = match self.blkio.as_ref() {
            Some(blkio) => self
                .read_optional(blkio, "blkio.throttle.io_service_bytes_recursive")?
                .map(|raw| parse_blkio_service_bytes(&raw))
                .transpose()?,
            None => None,
        };
        let (pids_current, pids_peak) = match self.pids.as_ref() {
            Some(pids) => (
                self.read_optional(pids, "pids.current")?
                    .map(|raw| parse_single_u64(&raw, "pids.current"))
                    .transpose()?,
                self.read_optional(pids, "pids.peak")?
                    .map(|raw| parse_single_u64(&raw, "pids.peak"))
                    .transpose()?,
            ),
            None => (None, None),
        };
        Ok(CgroupCounters {
            memory_current_bytes,
            memory_peak_bytes,
            memory_anon_bytes: stat("total_rss"),
            memory_file_bytes: stat("total_cache"),
            memory_swap_current_bytes: stat("total_swap"),
            memory_events: oom_kill.map(|oom_kill| MemoryEvents {
                oom_kill: Some(oom_kill),
                ..MemoryEvents::default()
            }),
            memory_events_local: None,
            cpu: CpuStat {
                usage_usec: usage_ns / 1_000,
                user_usec: user_ns.map(|ns| ns / 1_000),
                system_usec: system_ns.map(|ns| ns / 1_000),
                ..CpuStat::default()
            },
            io,
            pids_current,
            pids_peak,
        })
    }

    fn read(&self, directory: &File, name: &'static str) -> Result<String, CgroupError> {
        let descriptor = self.open(directory, name)?;
        read_descriptor(&descriptor, &self.display_path.join(name), name)
    }

    fn read_optional(
        &self,
        directory: &File,
        name: &'static str,
    ) -> Result<Option<String>, CgroupError> {
        match self.open(directory, name) {
            Ok(descriptor) => {
                read_descriptor(&descriptor, &self.display_path.join(name), name).map(Some)
            }
            Err(CgroupError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    fn open(&self, directory: &File, name: &'static str) -> Result<File, CgroupError> {
        let encoded = CString::new(name).expect("static cgroup filename contains no NUL");
        let descriptor = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                encoded.as_ptr(),
                libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        if descriptor < 0 {
            return Err(CgroupError::Io {
                operation: name,
                path: self.display_path.join(name),
                source: io::Error::last_os_error(),
            });
        }
        Ok(unsafe { File::from_raw_fd(descriptor) })
    }
}

/// Sums `Read` and `Write` over devices; the trailing `Total` line is ignored.
pub fn parse_blkio_service_bytes(raw: &str) -> Result<IoStat, CgroupError> {
    const FILE: &str = "blkio.throttle.io_service_bytes_recursive";
    let mut stat = IoStat {
        read_bytes: Some(0),
        write_bytes: Some(0),
        ..IoStat::default()
    };
    for (line_index, line) in raw.lines().enumerate() {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        match fields.as_slice() {
            [] | ["Total", _] => {}
            [_, "Read", value] => add_counter(&mut stat.read_bytes, value, FILE, "Read")?,
            [_, "Write", value] => add_counter(&mut stat.write_bytes, value, FILE, "Write")?,
            [device, key, value] => {
                stat.unknown
                    .insert(format!("{device}.{key}"), (*value).to_string());
            }
            _ => return malformed(FILE, line_index, "expected device, key and value"),
        }
    }
    Ok(stat)
}
