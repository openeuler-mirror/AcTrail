//! User-space process membership truth and inheritance rules.

use std::collections::BTreeMap;

use model_core::process::{MembershipState, ProcessIdentity, ProcessMembership};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MembershipIndex {
    by_identity: BTreeMap<ProcessIdentity, ProcessMembership>,
}

impl MembershipIndex {
    pub fn new(root: ProcessMembership) -> Self {
        let mut by_identity = BTreeMap::new();
        by_identity.insert(root.identity.clone(), root);
        Self { by_identity }
    }

    pub fn insert(&mut self, membership: ProcessMembership) {
        self.by_identity
            .insert(membership.identity.clone(), membership);
    }

    pub fn get(&self, identity: &ProcessIdentity) -> Option<&ProcessMembership> {
        self.by_identity.get(identity)
    }

    pub fn get_mut(&mut self, identity: &ProcessIdentity) -> Option<&mut ProcessMembership> {
        self.by_identity.get_mut(identity)
    }

    pub fn activate_all(&mut self) {
        for membership in self.by_identity.values_mut() {
            membership.activate();
        }
    }

    pub fn active_descendants_of(&self, root_identity: &ProcessIdentity) -> usize {
        self.by_identity
            .values()
            .filter(|membership| {
                membership.identity != *root_identity
                    && matches!(
                        membership.state,
                        MembershipState::Starting | MembershipState::Active
                    )
                    && membership.capture_enabled
            })
            .count()
    }

    pub fn capturable_members(&self) -> usize {
        self.by_identity
            .values()
            .filter(|membership| {
                membership.capture_enabled
                    && matches!(
                        membership.state,
                        MembershipState::Starting | MembershipState::Active
                    )
            })
            .count()
    }

    pub fn memberships(&self) -> impl Iterator<Item = &ProcessMembership> {
        self.by_identity.values()
    }
}
