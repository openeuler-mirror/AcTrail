//! User-space process membership truth and inheritance rules.

use std::collections::BTreeMap;

use model_core::process::{MembershipState, ProcessIdentity, ProcessMembership};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MembershipInsertResult {
    Inserted,
    PidReused { stale_identity: ProcessIdentity },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MembershipRefreshResult {
    Missing,
    Unchanged,
    Refreshed { stale_identity: ProcessIdentity },
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MembershipIndex {
    by_identity: BTreeMap<ProcessIdentity, ProcessMembership>,
    pid_to_identity: BTreeMap<u32, ProcessIdentity>,
}

impl MembershipIndex {
    pub fn new(root: ProcessMembership) -> Self {
        let mut by_identity = BTreeMap::new();
        let mut pid_to_identity = BTreeMap::new();
        pid_to_identity.insert(root.identity.pid, root.identity.clone());
        by_identity.insert(root.identity.clone(), root);
        Self {
            by_identity,
            pid_to_identity,
        }
    }

    pub fn insert(&mut self, membership: ProcessMembership) -> MembershipInsertResult {
        let pid = membership.identity.pid;
        let previous_identity = self
            .pid_to_identity
            .insert(pid, membership.identity.clone());
        self.by_identity
            .insert(membership.identity.clone(), membership);

        match previous_identity {
            Some(identity) if self.by_identity.contains_key(&identity) && identity.pid == pid => {
                if identity != *self.pid_to_identity.get(&pid).expect("pid just inserted") {
                    MembershipInsertResult::PidReused {
                        stale_identity: identity,
                    }
                } else {
                    MembershipInsertResult::Inserted
                }
            }
            _ => MembershipInsertResult::Inserted,
        }
    }

    pub fn get(&self, identity: &ProcessIdentity) -> Option<&ProcessMembership> {
        self.by_identity.get(identity)
    }

    pub fn get_mut(&mut self, identity: &ProcessIdentity) -> Option<&mut ProcessMembership> {
        self.by_identity.get_mut(identity)
    }

    pub fn by_pid(&self, pid: u32) -> Option<&ProcessMembership> {
        self.pid_to_identity
            .get(&pid)
            .and_then(|identity| self.by_identity.get(identity))
    }

    pub fn refresh_active_pid_identity(
        &mut self,
        refreshed_identity: ProcessIdentity,
    ) -> MembershipRefreshResult {
        let Some(current_identity) = self.pid_to_identity.get(&refreshed_identity.pid).cloned()
        else {
            return MembershipRefreshResult::Missing;
        };
        if current_identity == refreshed_identity {
            return MembershipRefreshResult::Unchanged;
        }
        let Some(current) = self.by_identity.get_mut(&current_identity) else {
            return MembershipRefreshResult::Missing;
        };
        if !current.can_inherit() {
            return MembershipRefreshResult::Missing;
        }

        let mut refreshed = current.clone();
        refreshed.identity = refreshed_identity.clone();
        refreshed.activate();
        current.disable_capture();
        current.disable_propagation();
        current.mark_identity_stale();

        self.by_identity
            .insert(refreshed_identity.clone(), refreshed);
        self.pid_to_identity
            .insert(refreshed_identity.pid, refreshed_identity);
        MembershipRefreshResult::Refreshed {
            stale_identity: current_identity,
        }
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

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use model_core::ids::TraceId;
    use model_core::process::{MembershipState, ProcessIdentity, ProcessMembership};

    use crate::membership::{MembershipIndex, MembershipRefreshResult};

    #[test]
    fn refresh_active_pid_identity_preserves_process_lineage() {
        let trace_id = TraceId::new(1);
        let parent = ProcessIdentity::new(10, 100, 100);
        let fork_identity = ProcessIdentity::new(11, 110, 110);
        let exec_identity = ProcessIdentity::new(11, 110, 111);
        let mut index = MembershipIndex::new(ProcessMembership::root(
            trace_id,
            parent.clone(),
            SystemTime::UNIX_EPOCH,
        ));
        let mut child = ProcessMembership::inherited(
            trace_id,
            fork_identity.clone(),
            parent,
            SystemTime::UNIX_EPOCH,
        );
        child.activate();
        index.insert(child);

        let result = index.refresh_active_pid_identity(exec_identity.clone());

        assert_eq!(
            result,
            MembershipRefreshResult::Refreshed {
                stale_identity: fork_identity.clone()
            }
        );
        assert_eq!(index.by_pid(11).unwrap().identity, exec_identity);
        assert_eq!(index.by_pid(11).unwrap().state, MembershipState::Active);
        assert_eq!(
            index.get(&fork_identity).unwrap().state,
            MembershipState::IdentityStale
        );
    }
}
