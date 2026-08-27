//! Lock-free publication gate and generation-tagged observation queue.

use std::num::NonZeroU64;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, TrySendError};

use sandbox_observation::Observation;

use crate::session::SessionWake;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ConnectionGeneration(NonZeroU64);

impl ConnectionGeneration {
    pub(super) fn new(value: u64) -> Option<Self> {
        NonZeroU64::new(value).map(Self)
    }

    pub(super) fn get(self) -> u64 {
        self.0.get()
    }
}

pub(super) struct ConnectionGate {
    generation: AtomicU64,
}

impl ConnectionGate {
    pub(super) fn disconnected() -> Self {
        Self {
            generation: AtomicU64::new(0),
        }
    }

    pub(super) fn generation(&self) -> Option<ConnectionGeneration> {
        ConnectionGeneration::new(self.generation.load(Ordering::Acquire))
    }

    pub(super) fn disable(&self) {
        self.generation.store(0, Ordering::Release);
    }

    pub(super) fn enable(&self, generation: ConnectionGeneration) {
        self.generation.store(generation.get(), Ordering::Release);
    }
}

pub(super) struct DeliveryEnvelope {
    pub(super) generation: ConnectionGeneration,
    pub(super) observation: Observation,
}

#[derive(Clone)]
pub(super) struct DeliveryPipeline {
    gate: Arc<ConnectionGate>,
    sender: SyncSender<DeliveryEnvelope>,
    wake: Arc<SessionWake>,
}

impl DeliveryPipeline {
    pub(super) fn new(
        gate: Arc<ConnectionGate>,
        sender: SyncSender<DeliveryEnvelope>,
        wake: Arc<SessionWake>,
    ) -> Self {
        Self { gate, sender, wake }
    }

    pub(super) fn capture_generation(&self) -> Option<ConnectionGeneration> {
        self.gate.generation()
    }

    pub(super) fn generation_is_current(&self, expected: ConnectionGeneration) -> bool {
        self.gate.generation() == Some(expected)
    }

    pub(super) fn publish_for(
        &self,
        expected: ConnectionGeneration,
        observation: Observation,
    ) -> DeliveryOutcome {
        if self.gate.generation() != Some(expected) {
            return DeliveryOutcome::Dropped;
        }
        match self.sender.try_send(DeliveryEnvelope {
            generation: expected,
            observation,
        }) {
            Ok(()) => {
                self.wake.notify();
                DeliveryOutcome::Accepted
            }
            Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {
                DeliveryOutcome::Dropped
            }
        }
    }

    pub(super) fn publish_iter<I>(
        &self,
        expected: ConnectionGeneration,
        observation_count: usize,
        observations: I,
    ) -> DeliveryCounts
    where
        I: IntoIterator<Item = Observation>,
        I::IntoIter: ExactSizeIterator,
    {
        if self.gate.generation() != Some(expected) {
            return DeliveryCounts::all_dropped(observation_count);
        }
        let mut counts = DeliveryCounts::default();
        let mut observations = observations.into_iter();
        while let Some(observation) = observations.next() {
            if self.gate.generation() != Some(expected) {
                counts.dropped += 1 + observations.len() as u64;
                break;
            }
            match self.sender.try_send(DeliveryEnvelope {
                generation: expected,
                observation,
            }) {
                Ok(()) => {
                    counts.accepted += 1;
                    self.wake.notify();
                }
                Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {
                    counts.dropped += 1;
                }
            }
        }
        counts
    }
}

pub(super) struct DeliveryQueue {
    receiver: Receiver<DeliveryEnvelope>,
}

impl DeliveryQueue {
    pub(super) fn new(receiver: Receiver<DeliveryEnvelope>) -> Self {
        Self { receiver }
    }

    pub(super) fn try_recv(&self) -> Result<DeliveryEnvelope, TryRecvError> {
        self.receiver.try_recv()
    }

    pub(super) fn discard_all(&self) {
        while self.receiver.try_recv().is_ok() {}
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DeliveryOutcome {
    Accepted,
    Dropped,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct DeliveryCounts {
    pub(super) accepted: u64,
    pub(super) dropped: u64,
}

impl DeliveryCounts {
    pub(super) fn all_dropped(count: usize) -> Self {
        Self {
            accepted: 0,
            dropped: count as u64,
        }
    }
}
