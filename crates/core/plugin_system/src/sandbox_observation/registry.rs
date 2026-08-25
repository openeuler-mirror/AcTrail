use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use arc_swap::ArcSwap;
use sandbox_plugin_delivery::{
    SandboxConsumerBatch, SandboxConsumerId, SandboxObservationConsumer, SandboxObservationKind,
    SandboxPluginIntentMatcher, SandboxPluginPublisher, SandboxRegistryGeneration,
};

use super::worker::SandboxConsumerWorker;

#[derive(Clone)]
pub struct SandboxPluginFacade {
    pub(super) registry: Arc<SandboxPluginRegistry>,
}

impl SandboxPluginFacade {
    pub fn new() -> Self {
        Self {
            registry: Arc::new(SandboxPluginRegistry::new()),
        }
    }

    pub fn matcher(&self) -> Arc<dyn SandboxPluginIntentMatcher> {
        Arc::new(self.clone())
    }

    pub fn publisher(&self) -> Arc<dyn SandboxPluginPublisher> {
        Arc::new(self.clone())
    }

    pub fn register(
        &self,
        registration: SandboxConsumerRegistration,
    ) -> Result<SandboxConsumerId, SandboxPluginRegistrationError> {
        self.registry.register(registration)
    }

    pub fn unregister(&self, consumer_id: SandboxConsumerId) -> SandboxPluginUnregisterResult {
        self.registry.unregister(consumer_id)
    }

    pub fn generation(&self) -> SandboxRegistryGeneration {
        self.registry.snapshot.load().generation
    }

    pub fn consumer_statuses(&self) -> Vec<SandboxConsumerStatus> {
        self.registry.consumer_statuses()
    }
}

impl Default for SandboxPluginFacade {
    fn default() -> Self {
        Self::new()
    }
}

pub struct SandboxConsumerRegistration {
    name: String,
    kinds: Box<[SandboxObservationKind]>,
    queue_capacity: u32,
    consumer: Arc<dyn SandboxObservationConsumer>,
}

impl SandboxConsumerRegistration {
    pub fn new(
        name: impl Into<String>,
        kinds: impl Into<Box<[SandboxObservationKind]>>,
        queue_capacity: u32,
        consumer: Arc<dyn SandboxObservationConsumer>,
    ) -> Self {
        Self {
            name: name.into(),
            kinds: kinds.into(),
            queue_capacity,
            consumer,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SandboxPluginRegistrationError {
    EmptyName,
    EmptySelector,
    DuplicateKind(SandboxObservationKind),
    ZeroQueueCapacity,
    QueueCapacityOverflow,
    ConsumerIdExhausted,
    RegistryGenerationExhausted,
    RegistryUnavailable,
    WorkerSpawnFailed(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SandboxPluginUnregisterResult {
    Unregistered {
        consumer_id: SandboxConsumerId,
        generation: SandboxRegistryGeneration,
    },
    NotFound {
        consumer_id: SandboxConsumerId,
        generation: SandboxRegistryGeneration,
    },
    RegistryGenerationExhausted,
    RegistryUnavailable,
    WorkerPanicked {
        consumer_id: SandboxConsumerId,
        generation: SandboxRegistryGeneration,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SandboxConsumerStatus {
    pub consumer_id: SandboxConsumerId,
    pub name: String,
    pub queue_depth: u64,
    pub queue_capacity: u32,
    pub observed_records: u64,
    pub dropped_records: u64,
    pub closed: bool,
    pub last_error: Option<String>,
}

pub(super) struct SandboxPluginRegistry {
    pub(super) snapshot: ArcSwap<RegistrySnapshot>,
    update: Mutex<RegistryState>,
}

impl SandboxPluginRegistry {
    fn new() -> Self {
        Self {
            snapshot: ArcSwap::from_pointee(RegistrySnapshot::empty()),
            update: Mutex::new(RegistryState {
                generation: SandboxRegistryGeneration::INITIAL,
                next_consumer_id: 1,
                consumers: BTreeMap::new(),
            }),
        }
    }

    fn register(
        &self,
        registration: SandboxConsumerRegistration,
    ) -> Result<SandboxConsumerId, SandboxPluginRegistrationError> {
        if registration.name.trim().is_empty() {
            return Err(SandboxPluginRegistrationError::EmptyName);
        }
        if registration.queue_capacity == 0 {
            return Err(SandboxPluginRegistrationError::ZeroQueueCapacity);
        }
        let queue_capacity = usize::try_from(registration.queue_capacity)
            .map_err(|_| SandboxPluginRegistrationError::QueueCapacityOverflow)?;
        let selector = CompiledKindSelector::compile(&registration.kinds)?;
        let mut state = self
            .update
            .lock()
            .map_err(|_| SandboxPluginRegistrationError::RegistryUnavailable)?;
        let generation = next_generation(state.generation)
            .ok_or(SandboxPluginRegistrationError::RegistryGenerationExhausted)?;
        let consumer_id = SandboxConsumerId::new(state.next_consumer_id);
        let next_consumer_id = state
            .next_consumer_id
            .checked_add(1)
            .ok_or(SandboxPluginRegistrationError::ConsumerIdExhausted)?;
        let (sender, receiver) = sync_channel(queue_capacity);
        let metrics = Arc::new(ConsumerMetrics::default());
        let worker = SandboxConsumerWorker::spawn(
            consumer_id,
            registration.name.clone(),
            registration.consumer,
            receiver,
            Arc::clone(&metrics),
        )
        .map_err(|error| SandboxPluginRegistrationError::WorkerSpawnFailed(error.to_string()))?;
        let endpoint = Arc::new(ConsumerEndpoint {
            consumer_id,
            name: registration.name,
            selector,
            queue_capacity: registration.queue_capacity,
            sender,
            metrics,
        });
        state.next_consumer_id = next_consumer_id;
        state.generation = generation;
        state
            .consumers
            .insert(consumer_id, ConsumerRuntime { endpoint, worker });
        self.snapshot.store(Arc::new(state.snapshot()));
        Ok(consumer_id)
    }

    fn unregister(&self, consumer_id: SandboxConsumerId) -> SandboxPluginUnregisterResult {
        let Ok(mut state) = self.update.lock() else {
            return SandboxPluginUnregisterResult::RegistryUnavailable;
        };
        let Some(generation) = next_generation(state.generation) else {
            return SandboxPluginUnregisterResult::RegistryGenerationExhausted;
        };
        let Some(runtime) = state.consumers.remove(&consumer_id) else {
            return SandboxPluginUnregisterResult::NotFound {
                consumer_id,
                generation: state.generation,
            };
        };
        state.generation = generation;
        self.snapshot.store(Arc::new(state.snapshot()));
        drop(state);
        if runtime.join().is_err() {
            return SandboxPluginUnregisterResult::WorkerPanicked {
                consumer_id,
                generation,
            };
        }
        SandboxPluginUnregisterResult::Unregistered {
            consumer_id,
            generation,
        }
    }

    fn consumer_statuses(&self) -> Vec<SandboxConsumerStatus> {
        self.snapshot
            .load()
            .endpoints
            .values()
            .map(|endpoint| endpoint.status())
            .collect()
    }
}

pub(super) struct RegistrySnapshot {
    pub(super) generation: SandboxRegistryGeneration,
    pub(super) endpoints: BTreeMap<SandboxConsumerId, Arc<ConsumerEndpoint>>,
    consumers_by_kind: [Box<[SandboxConsumerId]>; SandboxObservationKind::COUNT],
}

impl RegistrySnapshot {
    fn empty() -> Self {
        Self {
            generation: SandboxRegistryGeneration::INITIAL,
            endpoints: BTreeMap::new(),
            consumers_by_kind: [Box::new([]), Box::new([]), Box::new([])],
        }
    }

    pub(super) fn consumers_for(&self, kind: SandboxObservationKind) -> &[SandboxConsumerId] {
        &self.consumers_by_kind[kind.index()]
    }
}

struct RegistryState {
    generation: SandboxRegistryGeneration,
    next_consumer_id: u64,
    consumers: BTreeMap<SandboxConsumerId, ConsumerRuntime>,
}

impl RegistryState {
    fn snapshot(&self) -> RegistrySnapshot {
        let mut consumers_by_kind = [Vec::new(), Vec::new(), Vec::new()];
        let mut endpoints = BTreeMap::new();
        for (consumer_id, runtime) in &self.consumers {
            for kind in [
                SandboxObservationKind::ProcessIo,
                SandboxObservationKind::GuestResource,
                SandboxObservationKind::OomVictim,
            ] {
                if runtime.endpoint.selector.matches(kind) {
                    consumers_by_kind[kind.index()].push(*consumer_id);
                }
            }
            endpoints.insert(*consumer_id, Arc::clone(&runtime.endpoint));
        }
        RegistrySnapshot {
            generation: self.generation,
            endpoints,
            consumers_by_kind: consumers_by_kind.map(Vec::into_boxed_slice),
        }
    }
}

struct ConsumerRuntime {
    endpoint: Arc<ConsumerEndpoint>,
    worker: JoinHandle<()>,
}

impl ConsumerRuntime {
    fn join(self) -> std::thread::Result<()> {
        let Self { endpoint, worker } = self;
        drop(endpoint);
        worker.join()
    }
}

pub(super) struct ConsumerEndpoint {
    consumer_id: SandboxConsumerId,
    name: String,
    selector: CompiledKindSelector,
    queue_capacity: u32,
    sender: SyncSender<SandboxConsumerBatch>,
    metrics: Arc<ConsumerMetrics>,
}

impl ConsumerEndpoint {
    pub(super) fn selector_matches(&self, kind: SandboxObservationKind) -> bool {
        self.selector.matches(kind)
    }

    pub(super) fn sender(&self) -> SyncSender<SandboxConsumerBatch> {
        self.sender.clone()
    }

    pub(super) fn metrics(&self) -> &Arc<ConsumerMetrics> {
        &self.metrics
    }

    fn status(&self) -> SandboxConsumerStatus {
        SandboxConsumerStatus {
            consumer_id: self.consumer_id,
            name: self.name.clone(),
            queue_depth: self.metrics.queue_depth.load(Ordering::Relaxed),
            queue_capacity: self.queue_capacity,
            observed_records: self.metrics.observed_records.load(Ordering::Relaxed),
            dropped_records: self.metrics.dropped_records.load(Ordering::Relaxed),
            closed: self.metrics.closed.load(Ordering::Relaxed),
            last_error: self
                .metrics
                .last_error
                .lock()
                .ok()
                .and_then(|error| error.clone()),
        }
    }
}

pub(super) struct ConsumerMetrics {
    pub(super) queue_depth: AtomicU64,
    pub(super) observed_records: AtomicU64,
    pub(super) dropped_records: AtomicU64,
    pub(super) closed: AtomicBool,
    pub(super) last_error: Mutex<Option<String>>,
}

impl Default for ConsumerMetrics {
    fn default() -> Self {
        Self {
            queue_depth: AtomicU64::new(0),
            observed_records: AtomicU64::new(0),
            dropped_records: AtomicU64::new(0),
            closed: AtomicBool::new(false),
            last_error: Mutex::new(None),
        }
    }
}

#[derive(Clone, Copy)]
struct CompiledKindSelector(u8);

impl CompiledKindSelector {
    fn compile(kinds: &[SandboxObservationKind]) -> Result<Self, SandboxPluginRegistrationError> {
        if kinds.is_empty() {
            return Err(SandboxPluginRegistrationError::EmptySelector);
        }
        let mut mask = 0_u8;
        for kind in kinds {
            let bit = 1_u8 << kind.index();
            if mask & bit != 0 {
                return Err(SandboxPluginRegistrationError::DuplicateKind(*kind));
            }
            mask |= bit;
        }
        Ok(Self(mask))
    }

    const fn matches(self, kind: SandboxObservationKind) -> bool {
        self.0 & (1_u8 << kind.index()) != 0
    }
}

fn next_generation(generation: SandboxRegistryGeneration) -> Option<SandboxRegistryGeneration> {
    generation
        .get()
        .checked_add(1)
        .map(SandboxRegistryGeneration::new)
}
