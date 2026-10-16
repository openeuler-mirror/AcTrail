use crate::{ExternalScopePersistence, ManagedScopePersistence};
pub trait CgroupV2Persistence: ManagedScopePersistence + ExternalScopePersistence {}
impl<T: ManagedScopePersistence + ExternalScopePersistence> CgroupV2Persistence for T {}
