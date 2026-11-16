use super::{
    common::managed::ManagedScopePersistence, v1::CgroupV1BindingPersistence,
    v2::CgroupV2BindingPersistence,
};
use crate::ProfilingError;
pub trait CgroupPersistence:
    ManagedScopePersistence + CgroupV1BindingPersistence + CgroupV2BindingPersistence
{
    fn validate_recovery_versions(&mut self, v1: bool, v2: bool) -> Result<(), ProfilingError>;
}
