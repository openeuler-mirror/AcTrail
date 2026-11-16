use super::super::cgroup_error;
use super::ManagedHierarchy;
use crate::ProfilingError;
use crate::model::resource_scope::ResourceScopeLifecycleState;
use crate::model::resource_scope::TraceResourceScope;
use linux_cgroup::{ManagedScopeDirectory, TraceScopePaths};
use model_core::ids::TraceId;
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ResourceScopeRecovery {
    pub registered_missing: Vec<TraceId>,
    pub registered_invalid: Vec<TraceId>,
    pub unregistered_empty: Vec<ManagedScopeDirectory>,
    pub unregistered_populated: Vec<ManagedScopeDirectory>,
    pub terminal_empty: Vec<ManagedScopeDirectory>,
    pub terminal_populated: Vec<ManagedScopeDirectory>,
    pub unknown_directories: Vec<std::path::PathBuf>,
}

impl ResourceScopeRecovery {
    pub(super) fn reconcile<H: ManagedHierarchy>(
        adapter: &H,
        registered: Vec<TraceResourceScope>,
        scan: linux_cgroup::ManagedScopeScan,
    ) -> Result<
        (
            BTreeMap<TraceId, TraceScopePaths>,
            BTreeMap<TraceId, std::time::SystemTime>,
            BTreeSet<TraceId>,
            ResourceScopeRecovery,
        ),
        ProfilingError,
    > {
        let mut recovery = ResourceScopeRecovery {
            unknown_directories: scan.unknown,
            ..ResourceScopeRecovery::default()
        };
        let mut directories = BTreeMap::<u64, Vec<ManagedScopeDirectory>>::new();
        for directory in scan.managed {
            directories
                .entry(directory.trace_id)
                .or_default()
                .push(directory);
        }
        let mut scopes = BTreeMap::new();
        let mut waiting_since = BTreeMap::new();
        let mut finalized_barriers = BTreeSet::new();

        for record in registered {
            if !record.lifecycle_state.is_live() {
                finalized_barriers.insert(record.trace_id);
            }
            // A scope from the other hierarchy (for example, before a reboot that
            // changed the cgroup mode) cannot be recovered here.
            let parsed = match adapter.trace_paths_from_relative(&record.relative_path) {
                Ok(parsed)
                    if record.accounting_method == H::ACCOUNTING_METHOD
                        && parsed.trace_id == record.trace_id.get()
                        && parsed.nonce == record.nonce =>
                {
                    parsed
                }
                _ => {
                    if record.lifecycle_state.is_live() {
                        recovery.registered_invalid.push(record.trace_id);
                    }
                    continue;
                }
            };
            let Some(candidates) = directories.get_mut(&parsed.trace_id) else {
                if record.lifecycle_state.is_live() {
                    recovery.registered_missing.push(record.trace_id);
                }
                continue;
            };
            let Some(index) = candidates.iter().position(|directory| {
                directory.paths == parsed.paths && directory.nonce == parsed.nonce
            }) else {
                if record.lifecycle_state.is_live() {
                    recovery.registered_invalid.push(record.trace_id);
                }
                continue;
            };
            let directory = candidates.remove(index);
            let complete = adapter
                .recovery_scope_complete(&directory.paths)
                .map_err(cgroup_error("cgroup_recovery"))?;
            if record.lifecycle_state.is_live() && complete {
                if record.lifecycle_state == ResourceScopeLifecycleState::WaitingForEmpty {
                    waiting_since.insert(record.trace_id, record.updated_at);
                }
                scopes.insert(record.trace_id, directory.paths);
            } else {
                if record.lifecycle_state.is_live() {
                    recovery.registered_invalid.push(record.trace_id);
                }
                let empty = adapter
                    .subtree_is_empty(&directory.paths.aggregate)
                    .map_err(cgroup_error("cgroup_recovery"))?;
                if empty {
                    recovery.terminal_empty.push(directory);
                } else {
                    recovery.terminal_populated.push(directory);
                }
            }
        }

        for candidates in directories.into_values() {
            for directory in candidates {
                let empty = adapter
                    .subtree_is_empty(&directory.paths.aggregate)
                    .map_err(cgroup_error("cgroup_recovery"))?;
                if empty {
                    recovery.unregistered_empty.push(directory);
                } else {
                    recovery.unregistered_populated.push(directory);
                }
            }
        }
        recovery.registered_missing.sort();
        recovery.registered_invalid.sort();
        Ok((scopes, waiting_since, finalized_barriers, recovery))
    }
}
