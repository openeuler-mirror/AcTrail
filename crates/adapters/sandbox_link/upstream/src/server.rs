use std::io;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};

use gateway_ingest_runtime::{GatewayIngestRuntime, SandboxObservationSink};

use crate::config::UpstreamServerConfig;
use crate::connection::ConnectionWorker;
use crate::error::{ServerShutdownError, ServerStartError};
use crate::status::{ServerMetrics, UpstreamServerStatus};

pub struct UpstreamTcpServer {
    local_addr: SocketAddr,
    stop_requested: Arc<AtomicBool>,
    runtime: GatewayIngestRuntime,
    metrics: Arc<ServerMetrics>,
    accept_thread: Option<JoinHandle<()>>,
}

impl UpstreamTcpServer {
    pub fn start(
        config: UpstreamServerConfig,
        sink: Arc<dyn SandboxObservationSink>,
    ) -> Result<Self, ServerStartError> {
        config.validate()?;
        let listener = TcpListener::bind(config.listen_addr)
            .map_err(|error| ServerStartError::new("bind", error.to_string()))?;
        listener
            .set_nonblocking(true)
            .map_err(|error| ServerStartError::new("set_nonblocking", error.to_string()))?;
        let local_addr = listener
            .local_addr()
            .map_err(|error| ServerStartError::new("local_addr", error.to_string()))?;
        let runtime = GatewayIngestRuntime::new(config.max_connections, sink)
            .map_err(|error| ServerStartError::new("runtime", error.to_string()))?;
        let stop_requested = Arc::new(AtomicBool::new(false));
        let metrics = Arc::new(ServerMetrics::new());
        metrics.set_accepting(true);
        let accept_owner = AcceptOwner {
            listener,
            config,
            stop_requested: stop_requested.clone(),
            runtime: runtime.clone(),
            metrics: metrics.clone(),
            connections: Vec::new(),
        };
        let accept_thread = thread::Builder::new()
            .name("actrail-gateway-accept".to_string())
            .spawn(move || accept_owner.run())
            .map_err(|error| ServerStartError::new("accept_thread", error.to_string()))?;
        Ok(Self {
            local_addr,
            stop_requested,
            runtime,
            metrics,
            accept_thread: Some(accept_thread),
        })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub fn status(&self) -> UpstreamServerStatus {
        self.metrics
            .snapshot(self.local_addr, self.runtime.status())
    }

    pub fn shutdown(&mut self) -> Result<(), ServerShutdownError> {
        self.stop_requested.store(true, Ordering::Release);
        self.runtime.request_shutdown();
        let Some(thread) = self.accept_thread.take() else {
            return Ok(());
        };
        thread
            .join()
            .map_err(|_| ServerShutdownError::accept_thread_panicked())
    }
}

impl Drop for UpstreamTcpServer {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

struct AcceptOwner {
    listener: TcpListener,
    config: UpstreamServerConfig,
    stop_requested: Arc<AtomicBool>,
    runtime: GatewayIngestRuntime,
    metrics: Arc<ServerMetrics>,
    connections: Vec<JoinHandle<()>>,
}

impl AcceptOwner {
    fn run(mut self) {
        while !self.stop_requested.load(Ordering::Acquire) {
            self.reap_finished();
            match self.listener.accept() {
                Ok((stream, _)) => self.accept(stream),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(self.config.accept_poll_interval);
                }
                Err(_) => {
                    self.metrics.accept_failure();
                    thread::sleep(self.config.accept_poll_interval);
                }
            }
        }
        self.metrics.set_accepting(false);
        self.runtime.request_shutdown();
        self.join_connections();
    }

    fn accept(&mut self, stream: TcpStream) {
        self.metrics.accepted_socket();
        if self.connections.len() >= self.config.max_connections as usize {
            self.metrics.rejected_socket();
            return;
        }
        let worker = match ConnectionWorker::new(
            stream,
            self.runtime.clone(),
            self.metrics.clone(),
            &self.config,
        ) {
            Ok(worker) => worker,
            Err(_) => {
                self.metrics.connection_failure();
                return;
            }
        };
        match thread::Builder::new()
            .name("actrail-gateway-pending".to_string())
            .stack_size(self.config.connection_thread_stack_bytes)
            .spawn(move || worker.run())
        {
            Ok(handle) => self.connections.push(handle),
            Err(_) => self.metrics.spawn_failure(),
        }
    }

    fn reap_finished(&mut self) {
        let mut index = 0;
        while index < self.connections.len() {
            if self.connections[index].is_finished() {
                let handle = self.connections.swap_remove(index);
                if handle.join().is_err() {
                    self.metrics.connection_panic();
                }
            } else {
                index += 1;
            }
        }
    }

    fn join_connections(&mut self) {
        for handle in self.connections.drain(..) {
            if handle.join().is_err() {
                self.metrics.connection_panic();
            }
        }
    }
}
