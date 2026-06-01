//! Membership-write contracts.

use model_core::process::ProcessMembership;

use crate::WriteError;

pub trait MembershipWriteStore {
    fn upsert_membership(&mut self, membership: ProcessMembership) -> Result<(), WriteError>;
}
