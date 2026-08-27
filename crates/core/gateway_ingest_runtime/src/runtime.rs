use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use sandbox_observation::ObservationBatch;

use crate::{GatewayIngestStatus, SandboxObservationSink, SinkDeliveryError};

#[derive(Clone)]
pub struct GatewayIngestRuntime {
    inner: Arc<RuntimeState>,
}

struct RuntimeState {
    max_connections: u32,
    next_gateway_id: AtomicU64,
    shutdown_requested: AtomicBool,
    active_connections: AtomicU32,
    accepted_connections: AtomicU64,
    rejected_connections: AtomicU64,
    closed_connections: AtomicU64,
    heartbeats: AtomicU64,
    delivered_batches: AtomicU64,
    delivered_observations: AtomicU64,
    delivery_failures: AtomicU64,
    sink: Arc<dyn SandboxObservationSink>,
}

impl GatewayIngestRuntime {
    pub fn new(
        max_connections: u32,
        sink: Arc<dyn SandboxObservationSink>,
    ) -> Result<Self, GatewayOpenError> {
        if max_connections == 0 {
            return Err(GatewayOpenError::InvalidConnectionLimit);
        }
        Ok(Self {
            inner: Arc::new(RuntimeState {
                max_connections,
                next_gateway_id: AtomicU64::new(1),
                shutdown_requested: AtomicBool::new(false),
                active_connections: AtomicU32::new(0),
                accepted_connections: AtomicU64::new(0),
                rejected_connections: AtomicU64::new(0),
                closed_connections: AtomicU64::new(0),
                heartbeats: AtomicU64::new(0),
                delivered_batches: AtomicU64::new(0),
                delivered_observations: AtomicU64::new(0),
                delivery_failures: AtomicU64::new(0),
                sink,
            }),
        })
    }

    pub fn try_open(&self) -> Result<GatewayConnection, GatewayOpenError> {
        if self.inner.shutdown_requested.load(Ordering::Acquire) {
            return Err(GatewayOpenError::ShuttingDown);
        }
        self.inner
            .active_connections
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                (current < self.inner.max_connections).then_some(current + 1)
            })
            .map_err(|_| {
                self.inner
                    .rejected_connections
                    .fetch_add(1, Ordering::Relaxed);
                GatewayOpenError::Capacity
            })?;
        if self.inner.shutdown_requested.load(Ordering::Acquire) {
            self.release_reserved_connection();
            return Err(GatewayOpenError::ShuttingDown);
        }
        let raw_id = self.inner.next_gateway_id.fetch_add(1, Ordering::Relaxed);
        let gateway_id = match u32::try_from(raw_id) {
            Ok(id) if id != 0 => id,
            _ => {
                self.release_reserved_connection();
                return Err(GatewayOpenError::IdExhausted);
            }
        };
        self.inner
            .accepted_connections
            .fetch_add(1, Ordering::Relaxed);
        Ok(GatewayConnection {
            gateway_id,
            inner: self.inner.clone(),
        })
    }

    pub fn request_shutdown(&self) {
        self.inner.shutdown_requested.store(true, Ordering::Release);
    }

    pub fn is_shutdown_requested(&self) -> bool {
        self.inner.shutdown_requested.load(Ordering::Acquire)
    }

    pub fn status(&self) -> GatewayIngestStatus {
        GatewayIngestStatus {
            shutdown_requested: self.is_shutdown_requested(),
            active_connections: self.inner.active_connections.load(Ordering::Acquire),
            accepted_connections: self.inner.accepted_connections.load(Ordering::Relaxed),
            rejected_connections: self.inner.rejected_connections.load(Ordering::Relaxed),
            closed_connections: self.inner.closed_connections.load(Ordering::Relaxed),
            heartbeats: self.inner.heartbeats.load(Ordering::Relaxed),
            delivered_batches: self.inner.delivered_batches.load(Ordering::Relaxed),
            delivered_observations: self.inner.delivered_observations.load(Ordering::Relaxed),
            delivery_failures: self.inner.delivery_failures.load(Ordering::Relaxed),
        }
    }

    fn release_reserved_connection(&self) {
        self.inner.active_connections.fetch_sub(1, Ordering::AcqRel);
    }
}

pub struct GatewayConnection {
    gateway_id: u32,
    inner: Arc<RuntimeState>,
}

impl GatewayConnection {
    pub fn gateway_id(&self) -> u32 {
        self.gateway_id
    }

    pub fn record_heartbeat(&self) {
        self.inner.heartbeats.fetch_add(1, Ordering::Relaxed);
    }

    pub fn deliver(&self, sb_id: u32, batch: ObservationBatch) -> Result<(), SinkDeliveryError> {
        let observation_count = batch.observations.len() as u64;
        match self.inner.sink.deliver(self.gateway_id, sb_id, batch) {
            Ok(()) => {
                self.inner.delivered_batches.fetch_add(1, Ordering::Relaxed);
                self.inner
                    .delivered_observations
                    .fetch_add(observation_count, Ordering::Relaxed);
                Ok(())
            }
            Err(error) => {
                self.inner.delivery_failures.fetch_add(1, Ordering::Relaxed);
                Err(error)
            }
        }
    }
}

impl Drop for GatewayConnection {
    fn drop(&mut self) {
        self.inner.active_connections.fetch_sub(1, Ordering::AcqRel);
        self.inner
            .closed_connections
            .fetch_add(1, Ordering::Relaxed);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GatewayOpenError {
    InvalidConnectionLimit,
    Capacity,
    IdExhausted,
    ShuttingDown,
}

impl fmt::Display for GatewayOpenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidConnectionLimit => "gateway connection limit must be positive",
            Self::Capacity => "gateway connection capacity is full",
            Self::IdExhausted => "gateway ID space is exhausted",
            Self::ShuttingDown => "gateway ingest is shutting down",
        })
    }
}

impl std::error::Error for GatewayOpenError {}
