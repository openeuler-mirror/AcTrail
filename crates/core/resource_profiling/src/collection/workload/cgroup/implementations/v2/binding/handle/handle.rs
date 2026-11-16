use super::process_verifier::ProcfsProcessVerifier;
use super::{CgroupV2BindingError, ContainerCgroupIdentity};
use linux_cgroup::{CgroupCounters, CgroupV2CounterReader};
use model_core::process::HostProcessCoordinates;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CgroupDirectoryIdentity {
    pub device: u64,
    pub inode: u64,
}

#[derive(Debug)]
pub struct CgroupV2BindingHandle {
    pub identity: ContainerCgroupIdentity,
    pub relative_path: PathBuf,
    pub directory_identity: CgroupDirectoryIdentity,
    pub(super) reader: CgroupV2CounterReader,
}

impl CgroupV2BindingHandle {
    pub fn read_subtree_pids(&self) -> Result<Vec<u32>, linux_cgroup::CgroupError> {
        self.reader.read_subtree_pids()
    }
    pub fn read_counters(&mut self) -> Result<CgroupCounters, linux_cgroup::CgroupError> {
        self.reader.read_counters()
    }

    pub fn verify_membership(
        &self,
        procfs_root: &Path,
        coordinates: &HostProcessCoordinates,
    ) -> Result<(), CgroupV2BindingError> {
        ProcfsProcessVerifier::require_expected_start(coordinates)?;
        ProcfsProcessVerifier::require_start_time(procfs_root, coordinates)?;
        let raw = ProcfsProcessVerifier::read_proc_file(procfs_root, coordinates.pid, "cgroup")?;
        let current = ContainerCgroupIdentity::parse_container_cgroup_identity(&raw)?
            .ok_or(CgroupV2BindingError::ContainerIdentityMissing)?;
        if current.container_id != self.identity.container_id
            || current.runtime != self.identity.runtime
        {
            return Err(CgroupV2BindingError::ContainerIdentityMismatch);
        }
        if !ContainerCgroupIdentity::component_ancestor(
            &self.identity.container_boundary_path,
            &current.unified_process_path,
        ) {
            return Err(CgroupV2BindingError::RootMoved);
        }
        Ok(())
    }
}
