use super::{v1::CgroupV1Persistence, v2::CgroupV2Persistence};
use crate::ProfilingError;
pub trait CgroupPersistence: CgroupV1Persistence + CgroupV2Persistence {
    fn validate_recovery_versions(&mut self, v1: bool, v2: bool) -> Result<(), ProfilingError>;
}
