use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use gateway_ingest_runtime::GatewayIngestStatus;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UpstreamServerStatus {
    pub local_addr: SocketAddr,
    pub accepting: bool,
    pub accepted_sockets: u64,
    pub accept_failures: u64,
    pub rejected_sockets: u64,
    pub connection_spawn_failures: u64,
    pub connection_failures: u64,
    pub connection_panics: u64,
    pub sink_delivery_failed_batches: u64,
    pub gateway_ingest: GatewayIngestStatus,
}

pub(crate) struct ServerMetrics {
    accepting: AtomicBool,
    accepted_sockets: AtomicU64,
    accept_failures: AtomicU64,
    rejected_sockets: AtomicU64,
    connection_spawn_failures: AtomicU64,
    connection_failures: AtomicU64,
    connection_panics: AtomicU64,
    sink_delivery_failed_batches: AtomicU64,
}

impl ServerMetrics {
    pub(crate) fn new() -> Self {
        Self {
            accepting: AtomicBool::new(false),
            accepted_sockets: AtomicU64::new(0),
            accept_failures: AtomicU64::new(0),
            rejected_sockets: AtomicU64::new(0),
            connection_spawn_failures: AtomicU64::new(0),
            connection_failures: AtomicU64::new(0),
            connection_panics: AtomicU64::new(0),
            sink_delivery_failed_batches: AtomicU64::new(0),
        }
    }

    pub(crate) fn set_accepting(&self, value: bool) {
        self.accepting.store(value, Ordering::Release);
    }

    pub(crate) fn accepted_socket(&self) {
        self.accepted_sockets.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn accept_failure(&self) {
        self.accept_failures.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn rejected_socket(&self) {
        self.rejected_sockets.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn spawn_failure(&self) {
        self.connection_spawn_failures
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn connection_failure(&self) {
        self.connection_failures.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn connection_panic(&self) {
        self.connection_panics.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn sink_delivery_failed_batch(&self) {
        self.sink_delivery_failed_batches
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn snapshot(
        &self,
        local_addr: SocketAddr,
        gateway_ingest: GatewayIngestStatus,
    ) -> UpstreamServerStatus {
        UpstreamServerStatus {
            local_addr,
            accepting: self.accepting.load(Ordering::Acquire),
            accepted_sockets: self.accepted_sockets.load(Ordering::Relaxed),
            accept_failures: self.accept_failures.load(Ordering::Relaxed),
            rejected_sockets: self.rejected_sockets.load(Ordering::Relaxed),
            connection_spawn_failures: self.connection_spawn_failures.load(Ordering::Relaxed),
            connection_failures: self.connection_failures.load(Ordering::Relaxed),
            connection_panics: self.connection_panics.load(Ordering::Relaxed),
            sink_delivery_failed_batches: self.sink_delivery_failed_batches.load(Ordering::Relaxed),
            gateway_ingest,
        }
    }
}
