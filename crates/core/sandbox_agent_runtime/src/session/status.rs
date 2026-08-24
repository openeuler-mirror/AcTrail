use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering};

use sandbox_control::{
    SandboxConnectionState, SandboxControlStatus, SandboxDaemonState, SandboxEndpoint,
};

const DISCONNECTED: u8 = 0;
const CONNECTING: u8 = 1;
const CONNECTED: u8 = 2;
const RECONNECTING: u8 = 3;

pub(crate) struct SharedSessionStatus {
    revision: AtomicU64,
    stopping: AtomicBool,
    connection: AtomicU8,
    has_endpoint: AtomicBool,
    host_cid: AtomicU32,
    port: AtomicU32,
    sb_id: AtomicU32,
    generation: AtomicU64,
    publication_enabled: AtomicBool,
}

impl SharedSessionStatus {
    pub(crate) fn ready() -> Self {
        Self {
            revision: AtomicU64::new(0),
            stopping: AtomicBool::new(false),
            connection: AtomicU8::new(DISCONNECTED),
            has_endpoint: AtomicBool::new(false),
            host_cid: AtomicU32::new(0),
            port: AtomicU32::new(0),
            sb_id: AtomicU32::new(0),
            generation: AtomicU64::new(0),
            publication_enabled: AtomicBool::new(false),
        }
    }

    pub(super) fn connecting(&self, endpoint: SandboxEndpoint) {
        self.begin_update();
        self.set_endpoint(endpoint);
        self.sb_id.store(0, Ordering::Relaxed);
        self.publication_enabled.store(false, Ordering::Relaxed);
        self.connection.store(CONNECTING, Ordering::Release);
        self.finish_update();
    }

    pub(super) fn connected(&self, endpoint: SandboxEndpoint, sb_id: u32, generation: u64) {
        self.begin_update();
        self.set_endpoint(endpoint);
        self.sb_id.store(sb_id, Ordering::Relaxed);
        self.generation.store(generation, Ordering::Relaxed);
        self.publication_enabled.store(true, Ordering::Relaxed);
        self.connection.store(CONNECTED, Ordering::Release);
        self.finish_update();
    }

    pub(super) fn reconnecting(&self, endpoint: SandboxEndpoint) {
        self.begin_update();
        self.set_endpoint(endpoint);
        self.sb_id.store(0, Ordering::Relaxed);
        self.publication_enabled.store(false, Ordering::Relaxed);
        self.connection.store(RECONNECTING, Ordering::Release);
        self.finish_update();
    }

    pub(super) fn disconnected(&self) {
        self.begin_update();
        self.has_endpoint.store(false, Ordering::Relaxed);
        self.sb_id.store(0, Ordering::Relaxed);
        self.publication_enabled.store(false, Ordering::Relaxed);
        self.connection.store(DISCONNECTED, Ordering::Release);
        self.finish_update();
    }

    pub(crate) fn stopping(&self) {
        self.begin_update();
        self.stopping.store(true, Ordering::Release);
        self.publication_enabled.store(false, Ordering::Relaxed);
        self.finish_update();
    }

    pub(crate) fn snapshot(&self) -> SandboxControlStatus {
        loop {
            let revision = self.revision.load(Ordering::Acquire);
            if revision & 1 != 0 {
                std::hint::spin_loop();
                continue;
            }
            let raw_connection = self.connection.load(Ordering::Relaxed);
            let has_endpoint = self.has_endpoint.load(Ordering::Relaxed);
            let host_cid = self.host_cid.load(Ordering::Relaxed);
            let port = self.port.load(Ordering::Relaxed);
            let sb_id = self.sb_id.load(Ordering::Relaxed);
            let connection_generation = self.generation.load(Ordering::Relaxed);
            let publication_enabled = self.publication_enabled.load(Ordering::Relaxed);
            if self.revision.load(Ordering::Acquire) != revision {
                continue;
            }
            let connection = match raw_connection {
                CONNECTING => SandboxConnectionState::Connecting,
                CONNECTED => SandboxConnectionState::Connected,
                RECONNECTING => SandboxConnectionState::Reconnecting,
                _ => SandboxConnectionState::Disconnected,
            };
            let endpoint = has_endpoint.then(|| {
                SandboxEndpoint::new(host_cid, port)
                    .expect("stored sandbox endpoint was validated by its contract")
            });
            return SandboxControlStatus {
                daemon: if self.stopping.load(Ordering::Acquire) {
                    SandboxDaemonState::Stopping
                } else {
                    SandboxDaemonState::Ready
                },
                connection,
                endpoint,
                sb_id,
                connection_generation,
                publication_enabled,
            };
        }
    }

    fn set_endpoint(&self, endpoint: SandboxEndpoint) {
        self.host_cid.store(endpoint.host_cid(), Ordering::Relaxed);
        self.port.store(endpoint.port(), Ordering::Relaxed);
        self.has_endpoint.store(true, Ordering::Relaxed);
    }

    fn begin_update(&self) {
        let mut revision = self.revision.load(Ordering::Acquire);
        loop {
            if revision & 1 != 0 {
                std::hint::spin_loop();
                revision = self.revision.load(Ordering::Acquire);
                continue;
            }
            match self.revision.compare_exchange_weak(
                revision,
                revision.wrapping_add(1),
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return,
                Err(current) => revision = current,
            }
        }
    }

    fn finish_update(&self) {
        self.revision.fetch_add(1, Ordering::Release);
    }
}
