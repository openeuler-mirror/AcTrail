use super::super::CgroupV1Config;
use super::{directory::V1Directory, discovery::V1Discovery};
use crate::ProfilingError;
use crate::model::cgroup_v1_binding::CgroupV1Binding;
use crate::model::cgroup_v1_binding::CgroupV1Boundary;
use model_core::host_identity::HostBootId;
use model_core::ids::TraceId;
use model_core::process::HostProcessCoordinates;
use std::{collections::BTreeMap, path::PathBuf, time::SystemTime};
pub(crate) struct V1ProcessBinding {
    cpuacct: V1Directory,
    memory: V1Directory,
}
impl V1ProcessBinding {
    pub(crate) fn bind(
        discovery: &V1Discovery,
        config: &CgroupV1Config,
        id: TraceId,
        coordinates: &HostProcessCoordinates,
        boot: HostBootId,
    ) -> Result<(CgroupV1Binding, Self), ProfilingError> {
        for _ in 0..config.binding_attempts {
            Self::verify_process(coordinates)?;
            let before = V1Discovery::membership(coordinates.pid)?;
            let (cpuacct, cpu) = Self::bind_controller(discovery, config, &before, "cpuacct")?;
            let (memory, mem) = Self::bind_controller(discovery, config, &before, "memory")?;
            Self::verify_process(coordinates)?;
            if before != V1Discovery::membership(coordinates.pid)? {
                continue;
            }
            let binding = Self {
                cpuacct: cpu,
                memory: mem,
            };
            // Required metrics are proven readable before admission.
            binding.cpuacct.read("cpuacct.usage")?;
            binding.memory.read("memory.usage_in_bytes")?;
            return Ok((
                CgroupV1Binding {
                    trace_id: id,
                    host_boot_id: boot,
                    cpuacct,
                    memory,
                    created_at: SystemTime::now(),
                    stale_reason: None,

                    closed_at: None,
                },
                binding,
            ));
        }
        Err(V1Discovery::unavailable(
            "controller memberships changed during binding",
        ))
    }
    fn bind_controller(
        discovery: &V1Discovery,
        config: &CgroupV1Config,
        members: &BTreeMap<String, PathBuf>,
        controller: &str,
    ) -> Result<(CgroupV1Boundary, V1Directory), ProfilingError> {
        let mount = discovery.mount(controller)?;
        let membership = members.get(controller).ok_or_else(|| {
            V1Discovery::unavailable(format!("{controller} membership unavailable"))
        })?;
        let relative_path = V1Discovery::relative(membership, &mount.root)?;
        if relative_path.as_os_str().is_empty() && !config.allow_hierarchy_root {
            return Err(V1Discovery::unavailable(
                "hierarchy-root accounting is disabled",
            ));
        }
        let directory = V1Directory::open(&mount.point, &relative_path)?;
        let (device, inode) = directory.identity()?;
        Ok((
            CgroupV1Boundary {
                mount_point: mount.point.clone(),
                mount_root: mount.root.clone(),
                membership: membership.clone(),
                relative_path,
                device,
                inode,
            },
            directory,
        ))
    }
    pub(crate) fn reopen(
        discovery: &V1Discovery,
        binding: &CgroupV1Binding,
        boot: HostBootId,
    ) -> Result<Self, ProfilingError> {
        if binding.host_boot_id != boot {
            return Err(V1Discovery::invalid("host boot changed"));
        }
        Ok(Self {
            cpuacct: Self::reopen_controller(discovery, "cpuacct", &binding.cpuacct)?,
            memory: Self::reopen_controller(discovery, "memory", &binding.memory)?,
        })
    }
    fn reopen_controller(
        discovery: &V1Discovery,
        controller: &str,
        boundary: &CgroupV1Boundary,
    ) -> Result<V1Directory, ProfilingError> {
        let mount = discovery.mount(controller)?;
        if mount.point != boundary.mount_point || mount.root != boundary.mount_root {
            return Err(V1Discovery::invalid("controller mount identity changed"));
        }
        let directory = V1Directory::open(&mount.point, &boundary.relative_path)?;
        if directory.identity()? != (boundary.device, boundary.inode) {
            return Err(V1Discovery::invalid(
                "controller directory identity changed",
            ));
        }
        Ok(directory)
    }
    pub(crate) fn verify(
        &self,
        binding: &CgroupV1Binding,
        coordinates: &HostProcessCoordinates,
    ) -> Result<(), ProfilingError> {
        Self::verify_process(coordinates)?;
        let membership = V1Discovery::membership(coordinates.pid)?;
        if membership.get("cpuacct") != Some(&binding.cpuacct.membership)
            || membership.get("memory") != Some(&binding.memory.membership)
        {
            return Err(V1Discovery::invalid("root changed controller membership"));
        }
        for boundary in [&binding.cpuacct, &binding.memory] {
            let current = V1Directory::open(&boundary.mount_point, &boundary.relative_path)?;
            if current.identity()? != (boundary.device, boundary.inode) {
                return Err(V1Discovery::invalid(
                    "bound controller directory was replaced",
                ));
            }
        }
        Self::verify_process(coordinates)
    }
    fn verify_process(coordinates: &HostProcessCoordinates) -> Result<(), ProfilingError> {
        let text = std::fs::read_to_string(format!("/proc/{}/stat", coordinates.pid))
            .map_err(V1Discovery::io)?;
        let (_, fields) = text
            .rsplit_once(')')
            .ok_or_else(|| V1Discovery::invalid("malformed process stat"))?;
        let fields: Vec<_> = fields.split_whitespace().collect();
        let ticks = fields
            .get(19)
            .and_then(|v| v.parse::<u64>().ok())
            .ok_or_else(|| V1Discovery::invalid("process generation unavailable"))?;
        if ticks != coordinates.start_time_ticks
            || fields
                .first()
                .is_some_and(|state| matches!(*state, "Z" | "X" | "x"))
        {
            return Err(V1Discovery::invalid(
                "root process generation changed or exited",
            ));
        }
        Ok(())
    }
    pub(crate) fn cpu(&self, name: &str) -> Result<String, ProfilingError> {
        self.cpuacct.read(name)
    }
    pub(crate) fn memory(&self, name: &str) -> Result<String, ProfilingError> {
        self.memory.read(name)
    }
}
