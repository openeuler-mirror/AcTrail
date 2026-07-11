use std::collections::{BTreeMap, BTreeSet};

use crate::{
    HostProcessCoordinates, NamespaceIdentity, NamespaceProcessCoordinates, ProcessIdentity,
    ProcessObservation, ProcessRecord, ProcessResolutionState,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProcessIdentityError {
    IdOverflow,
    IdBlockExhausted,
    ConflictingObservations {
        identities: Vec<ProcessIdentity>,
        observation: ProcessObservation,
    },
    HostCoordinatesConflict {
        identity: ProcessIdentity,
        existing: HostProcessCoordinates,
        observed: HostProcessCoordinates,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessResolution {
    pub identity: ProcessIdentity,
    pub created: bool,
    pub enriched: bool,
    pub displaced_active: Vec<ProcessIdentity>,
}

#[derive(Debug)]
pub struct ProcessIdentityManager {
    next_identity: u64,
    identity_block_end: u64,
    records: BTreeMap<ProcessIdentity, ProcessRecord>,
    by_host_start_ticks: BTreeMap<(u32, u64), ProcessIdentity>,
    by_host_start_boottime: BTreeMap<(u32, u64), ProcessIdentity>,
    by_namespace: BTreeMap<NamespaceProcessCoordinates, ProcessIdentity>,
    active_by_host_pid: BTreeMap<u32, ProcessIdentity>,
    active_by_namespace_pid: BTreeMap<(NamespaceIdentity, u32), ProcessIdentity>,
}

impl ProcessIdentityManager {
    pub fn new(next_identity: u64) -> Self {
        Self {
            next_identity,
            identity_block_end: u64::MAX,
            records: BTreeMap::new(),
            by_host_start_ticks: BTreeMap::new(),
            by_host_start_boottime: BTreeMap::new(),
            by_namespace: BTreeMap::new(),
            active_by_host_pid: BTreeMap::new(),
            active_by_namespace_pid: BTreeMap::new(),
        }
    }

    pub fn with_reserved_block(
        block_start: u64,
        block_end: u64,
        records: impl IntoIterator<Item = ProcessRecord>,
    ) -> Result<Self, ProcessIdentityError> {
        if block_start == 0 || block_start >= block_end {
            return Err(ProcessIdentityError::IdBlockExhausted);
        }
        let mut registry = Self {
            next_identity: block_start,
            identity_block_end: block_end,
            records: BTreeMap::new(),
            by_host_start_ticks: BTreeMap::new(),
            by_host_start_boottime: BTreeMap::new(),
            by_namespace: BTreeMap::new(),
            active_by_host_pid: BTreeMap::new(),
            active_by_namespace_pid: BTreeMap::new(),
        };
        for record in records {
            registry.load_record(record)?;
        }
        Ok(registry)
    }

    pub fn install_reserved_block(
        &mut self,
        block_start: u64,
        block_end: u64,
    ) -> Result<(), ProcessIdentityError> {
        if self.next_identity != self.identity_block_end
            || block_start != self.identity_block_end
            || block_start >= block_end
        {
            return Err(ProcessIdentityError::IdBlockExhausted);
        }
        self.next_identity = block_start;
        self.identity_block_end = block_end;
        Ok(())
    }

    pub fn resolve_or_create(
        &mut self,
        observation: ProcessObservation,
    ) -> Result<ProcessResolution, ProcessIdentityError> {
        let candidates = self.candidates(&observation);
        if candidates.len() > 1 {
            return Err(ProcessIdentityError::ConflictingObservations {
                identities: candidates.into_iter().collect(),
                observation,
            });
        }

        let (identity, created) = match candidates.first().copied() {
            Some(identity) => (identity, false),
            None => (self.allocate_identity()?, true),
        };
        if created {
            self.records
                .insert(identity, ProcessRecord::new(identity, observation.clone()));
        }
        let enriched = if created {
            false
        } else {
            self.enrich(identity, &observation)?
        };
        let displaced_active = self.index(identity, &observation);
        Ok(ProcessResolution {
            identity,
            created,
            enriched,
            displaced_active,
        })
    }

    pub fn record(&self, identity: ProcessIdentity) -> Option<&ProcessRecord> {
        self.records.get(&identity)
    }

    pub fn lookup(
        &self,
        observation: &ProcessObservation,
    ) -> Result<Option<ProcessIdentity>, ProcessIdentityError> {
        let candidates = self.candidates(observation);
        if candidates.len() > 1 {
            return Err(ProcessIdentityError::ConflictingObservations {
                identities: candidates.into_iter().collect(),
                observation: observation.clone(),
            });
        }
        Ok(candidates.first().copied())
    }

    pub fn records(&self) -> impl Iterator<Item = &ProcessRecord> {
        self.records.values()
    }

    pub fn active_host_pid(&self, pid: u32) -> Option<ProcessIdentity> {
        self.active_by_host_pid.get(&pid).copied()
    }

    pub fn active_namespace_pid(
        &self,
        pid_namespace: &NamespaceIdentity,
        pid: u32,
    ) -> Option<ProcessIdentity> {
        self.active_by_namespace_pid
            .get(&(pid_namespace.clone(), pid))
            .copied()
    }

    pub fn mark_exited(&mut self, identity: ProcessIdentity) {
        let Some(record) = self.records.get(&identity) else {
            return;
        };
        if let Some(host) = &record.host
            && self.active_by_host_pid.get(&host.pid) == Some(&identity)
        {
            self.active_by_host_pid.remove(&host.pid);
        }
        for namespace in &record.namespaces {
            let key = (namespace.pid_namespace.clone(), namespace.pid);
            if self.active_by_namespace_pid.get(&key) == Some(&identity) {
                self.active_by_namespace_pid.remove(&key);
            }
        }
    }

    fn allocate_identity(&mut self) -> Result<ProcessIdentity, ProcessIdentityError> {
        if self.next_identity >= self.identity_block_end {
            return Err(ProcessIdentityError::IdBlockExhausted);
        }
        let raw = self.next_identity;
        self.next_identity = self
            .next_identity
            .checked_add(1)
            .ok_or(ProcessIdentityError::IdOverflow)?;
        Ok(ProcessIdentity::new(raw))
    }

    fn load_record(&mut self, record: ProcessRecord) -> Result<(), ProcessIdentityError> {
        let identity = record.identity;
        if let Some(existing) = self.records.get(&identity) {
            if existing == &record {
                return Ok(());
            }
            return Err(ProcessIdentityError::ConflictingObservations {
                identities: vec![identity],
                observation: ProcessObservation {
                    host: record.host,
                    namespace: record.namespaces.into_iter().next(),
                },
            });
        }
        if let Some(host) = &record.host {
            self.index_host_coordinates(identity, host);
        }
        for namespace in &record.namespaces {
            self.index_namespace_coordinates(identity, namespace);
        }
        self.records.insert(identity, record);
        Ok(())
    }

    fn candidates(&self, observation: &ProcessObservation) -> BTreeSet<ProcessIdentity> {
        let mut candidates = BTreeSet::new();
        if let Some(host) = &observation.host {
            if host.start_time_ticks != 0
                && let Some(identity) = self
                    .by_host_start_ticks
                    .get(&(host.pid, host.start_time_ticks))
            {
                candidates.insert(*identity);
            }
            if let Some(start_boottime_ns) = host.start_boottime_ns
                && let Some(identity) = self
                    .by_host_start_boottime
                    .get(&(host.pid, start_boottime_ns))
            {
                candidates.insert(*identity);
            }
            if let Some(identity) = self.active_by_host_pid.get(&host.pid)
                && self
                    .records
                    .get(identity)
                    .and_then(|record| record.host.as_ref())
                    .is_some_and(|existing| Self::host_coordinates_match(existing, host))
            {
                candidates.insert(*identity);
            }
        }
        if let Some(namespace) = &observation.namespace {
            if namespace.start_time_ticks != 0
                && let Some(identity) = self.by_namespace.get(namespace)
            {
                candidates.insert(*identity);
            }
            let active_key = (namespace.pid_namespace.clone(), namespace.pid);
            if let Some(identity) = self.active_by_namespace_pid.get(&active_key)
                && self.records.get(identity).is_some_and(|record| {
                    record
                        .namespaces
                        .iter()
                        .any(|existing| Self::namespace_coordinates_match(existing, namespace))
                })
            {
                candidates.insert(*identity);
            }
        }
        candidates
    }

    fn enrich(
        &mut self,
        identity: ProcessIdentity,
        observation: &ProcessObservation,
    ) -> Result<bool, ProcessIdentityError> {
        let record = self
            .records
            .get_mut(&identity)
            .expect("resolved identity must have a record");
        let mut enriched = false;
        if let Some(observed) = &observation.host {
            match &mut record.host {
                Some(existing) if !Self::host_coordinates_match(existing, observed) => {
                    record.resolution_state = ProcessResolutionState::Conflicted;
                    return Err(ProcessIdentityError::HostCoordinatesConflict {
                        identity,
                        existing: existing.clone(),
                        observed: observed.clone(),
                    });
                }
                Some(existing) => {
                    if existing.start_time_ticks == 0 && observed.start_time_ticks != 0 {
                        existing.start_time_ticks = observed.start_time_ticks;
                        enriched = true;
                    }
                    if existing.start_boottime_ns.is_none() && observed.start_boottime_ns.is_some()
                    {
                        existing.start_boottime_ns = observed.start_boottime_ns;
                        enriched = true;
                    }
                    if existing.task_id.is_none() && observed.task_id.is_some() {
                        existing.task_id = observed.task_id;
                        enriched = true;
                    }
                }
                None => {
                    record.host = Some(observed.clone());
                    enriched = true;
                }
            }
        }
        if let Some(namespace) = &observation.namespace
            && record.namespaces.insert(namespace.clone())
        {
            enriched = true;
        }
        if record.host.is_some() {
            record.resolution_state = ProcessResolutionState::Resolved;
        }
        if enriched {
            let record = record.clone();
            self.index_record_coordinates(&record);
        }
        Ok(enriched)
    }

    fn index(
        &mut self,
        identity: ProcessIdentity,
        observation: &ProcessObservation,
    ) -> Vec<ProcessIdentity> {
        let mut displaced = BTreeSet::new();
        if let Some(host) = &observation.host {
            self.index_host_coordinates(identity, host);
            if let Some(previous) = self.active_by_host_pid.insert(host.pid, identity)
                && previous != identity
            {
                displaced.insert(previous);
            }
        }
        if let Some(namespace) = &observation.namespace {
            self.index_namespace_coordinates(identity, namespace);
            let active_key = (namespace.pid_namespace.clone(), namespace.pid);
            if let Some(previous) = self.active_by_namespace_pid.insert(active_key, identity)
                && previous != identity
            {
                displaced.insert(previous);
            }
        }
        displaced.into_iter().collect()
    }

    fn index_record_coordinates(&mut self, record: &ProcessRecord) {
        if let Some(host) = &record.host {
            self.index_host_coordinates(record.identity, host);
        }
        for namespace in &record.namespaces {
            self.index_namespace_coordinates(record.identity, namespace);
        }
    }

    fn index_host_coordinates(&mut self, identity: ProcessIdentity, host: &HostProcessCoordinates) {
        if host.start_time_ticks != 0 {
            self.by_host_start_ticks
                .insert((host.pid, host.start_time_ticks), identity);
        }
        if let Some(start_boottime_ns) = host.start_boottime_ns {
            self.by_host_start_boottime
                .insert((host.pid, start_boottime_ns), identity);
        }
    }

    fn index_namespace_coordinates(
        &mut self,
        identity: ProcessIdentity,
        namespace: &NamespaceProcessCoordinates,
    ) {
        if namespace.start_time_ticks != 0 {
            self.by_namespace.insert(namespace.clone(), identity);
        }
    }

    fn host_coordinates_match(
        left: &HostProcessCoordinates,
        right: &HostProcessCoordinates,
    ) -> bool {
        left.pid == right.pid
            && (left.start_time_ticks == 0
                || right.start_time_ticks == 0
                || left.start_time_ticks == right.start_time_ticks)
            && match (left.start_boottime_ns, right.start_boottime_ns) {
                (Some(left), Some(right)) => left == right,
                _ => true,
            }
    }

    fn namespace_coordinates_match(
        left: &NamespaceProcessCoordinates,
        right: &NamespaceProcessCoordinates,
    ) -> bool {
        left.pid_namespace == right.pid_namespace
            && left.pid == right.pid
            && (left.start_time_ticks == 0
                || right.start_time_ticks == 0
                || left.start_time_ticks == right.start_time_ticks)
    }
}
