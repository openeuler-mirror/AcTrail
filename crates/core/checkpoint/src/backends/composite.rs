//! One backend made of the integrations that take part in a Trace. Each child has its own scopes;
//! a composite snapshot is one snapshot of every child, and failing on one child fails it as a
//! whole. Children run in order, and a child that fails stops the calls to those after it.

use std::collections::{BTreeMap, BTreeSet};

use abi_stable::std_types::{RResult, RVec};
use checkpoint::{
    BackendError, CheckpointBackend, RestorableCheckpointBackend, RestoreFailure, SnapshotContext,
    SnapshotId,
};

use crate::scope::BackendGroup;

pub(crate) struct Child {
    name: String,
    backend: CheckpointBackend,
    scopes: BTreeSet<String>,
}

impl Child {
    /// A child without a group works on no scopes.
    pub(crate) fn new(
        name: &str,
        group: Option<&BackendGroup>,
        backend: CheckpointBackend,
    ) -> Self {
        Self {
            name: name.to_string(),
            backend,
            scopes: group
                .iter()
                .flat_map(|group| &group.scopes)
                .map(|scope| scope.id.clone())
                .collect(),
        }
    }

    fn named(&self, error: BackendError) -> BackendError {
        BackendError::new(
            error.kind,
            format!("backend instance {}: {}", self.name, error.message),
        )
    }

    fn connect(&mut self, context: &SnapshotContext) -> Result<(), BackendError> {
        let own = SnapshotContext {
            scopes: context
                .scopes
                .iter()
                .filter(|scope| self.scopes.contains(scope.id.as_str()))
                .cloned()
                .collect(),
            ..context.clone()
        };
        self.backend
            .connect(&own)
            .into_result()
            .map_err(|error| self.named(error))
    }

    fn snapshot(&mut self) -> Result<SnapshotId, BackendError> {
        self.backend
            .snapshot()
            .into_result()
            .map_err(|error| self.named(error))
    }

    fn restore(&mut self, id: &SnapshotId) -> Result<RVec<SnapshotId>, RestoreFailure> {
        self.backend
            .restore(id)
            .into_result()
            .map_err(|failure| RestoreFailure {
                error: self.named(failure.error),
                invalidated: failure.invalidated,
            })
    }

    fn delete(&mut self, id: &SnapshotId) -> Result<(), BackendError> {
        self.backend
            .delete(id)
            .into_result()
            .map_err(|error| self.named(error))
    }

    fn disconnect(&mut self) -> Result<(), BackendError> {
        self.backend
            .disconnect()
            .into_result()
            .map_err(|error| self.named(error))
    }
}

pub(crate) struct CompositeBackend {
    children: Vec<Child>,
    /// The child snapshots behind each composite snapshot, as child index and child ID.
    snapshots: BTreeMap<u64, Vec<(usize, SnapshotId)>>,
    next_snapshot: u64,
}

impl CompositeBackend {
    pub(crate) fn new(children: Vec<Child>) -> Self {
        Self {
            children,
            snapshots: BTreeMap::new(),
            next_snapshot: 1,
        }
    }

    fn connect_children(&mut self, context: &SnapshotContext) -> Result<(), BackendError> {
        for index in 0..self.children.len() {
            if let Err(error) = self.children[index].connect(context) {
                // The launch fails, so a failure here only leaves space behind.
                for child in &mut self.children[..index] {
                    let _ = child.disconnect();
                }
                return Err(error);
            }
        }
        Ok(())
    }

    /// The child snapshots are of no use without the others, so a failure deletes them again; a
    /// delete that fails only leaves space behind until disconnect.
    fn take_snapshot(&mut self) -> Result<SnapshotId, BackendError> {
        let mut taken = Vec::with_capacity(self.children.len());
        for index in 0..self.children.len() {
            match self.children[index].snapshot() {
                Ok(id) => taken.push((index, id)),
                Err(error) => {
                    for (index, id) in &taken {
                        let _ = self.children[*index].delete(id);
                    }
                    return Err(error);
                }
            }
        }
        let id = self.next_snapshot;
        self.next_snapshot += 1;
        self.snapshots.insert(id, taken);
        Ok(snapshot_id(id))
    }

    fn roll_back(&mut self, id: &SnapshotId) -> Result<RVec<SnapshotId>, RestoreFailure> {
        let parts = self
            .snapshots
            .get(&parse_id(id)?)
            .cloned()
            .ok_or_else(|| unknown_snapshot(id))?;
        let mut invalidated = BTreeSet::new();
        for (index, child_id) in parts {
            let (lost, failure) = match self.children[index].restore(&child_id) {
                Ok(lost) => (lost, None),
                Err(failure) => (failure.invalidated, Some(failure.error)),
            };
            invalidated.extend(lost.iter().map(|id| (index, id.0.to_vec())));
            if let Some(error) = failure {
                return Err(RestoreFailure {
                    error,
                    invalidated: self.containing(&invalidated),
                });
            }
        }
        Ok(self.containing(&invalidated))
    }

    /// The composite snapshots that lost one of the given child snapshots.
    fn containing(&self, children: &BTreeSet<(usize, Vec<u8>)>) -> RVec<SnapshotId> {
        self.snapshots
            .iter()
            .filter(|(_, parts)| {
                parts
                    .iter()
                    .any(|(index, id)| children.contains(&(*index, id.0.to_vec())))
            })
            .map(|(id, _)| snapshot_id(*id))
            .collect()
    }

    /// A child snapshot already deleted leaves the record, so another delete retries only the
    /// rest.
    fn remove(&mut self, id: &SnapshotId) -> Result<(), BackendError> {
        let id = parse_id(id)?;
        let Some(parts) = self.snapshots.get_mut(&id) else {
            return Ok(());
        };
        while let Some((index, child_id)) = parts.last() {
            self.children[*index].delete(child_id)?;
            parts.pop();
        }
        self.snapshots.remove(&id);
        Ok(())
    }

    /// Every child frees what it holds, whichever of them fails; the first failure is reported.
    /// A child that disconnected leaves, so another call retries only the rest.
    fn disconnect_children(&mut self) -> Result<(), BackendError> {
        let mut first_failure = None;
        self.children.retain_mut(|child| match child.disconnect() {
            Ok(()) => false,
            Err(error) => {
                first_failure.get_or_insert(error);
                true
            }
        });
        first_failure.map_or(Ok(()), Err)
    }
}

impl RestorableCheckpointBackend for CompositeBackend {
    fn connect(&mut self, context: &SnapshotContext) -> RResult<(), BackendError> {
        self.connect_children(context).into()
    }

    fn snapshot(&mut self) -> RResult<SnapshotId, BackendError> {
        self.take_snapshot().into()
    }

    fn restore(&mut self, snapshot_id: &SnapshotId) -> RResult<RVec<SnapshotId>, RestoreFailure> {
        self.roll_back(snapshot_id).into()
    }

    fn delete(&mut self, snapshot_id: &SnapshotId) -> RResult<(), BackendError> {
        self.remove(snapshot_id).into()
    }

    fn disconnect(&mut self) -> RResult<(), BackendError> {
        self.disconnect_children().into()
    }
}

fn snapshot_id(id: u64) -> SnapshotId {
    SnapshotId(id.to_string().into_bytes().into())
}

fn parse_id(id: &SnapshotId) -> Result<u64, BackendError> {
    std::str::from_utf8(&id.0)
        .ok()
        .and_then(|text| text.parse().ok())
        .ok_or_else(|| unknown_snapshot(id))
}

fn unknown_snapshot(id: &SnapshotId) -> BackendError {
    BackendError::new(
        checkpoint::BackendErrorKind::InvalidSnapshot,
        format!("unknown snapshot {}", String::from_utf8_lossy(&id.0)),
    )
}
