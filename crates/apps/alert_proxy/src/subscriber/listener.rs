use std::io;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};

use socket2::{Domain, Protocol, Socket, Type};

use crate::diagnostics::ProxyDiagnostics;
use crate::registry::SubscriberRegistry;
use crate::startup::SubscriberConfig;

use super::session::SubscriberSession;

pub(crate) struct SubscriberServer {
    local_addr: SocketAddr,
    stop: Arc<AtomicBool>,
    registry: Arc<SubscriberRegistry>,
    thread: Option<JoinHandle<()>>,
}

impl SubscriberServer {
    pub(crate) fn start(
        config: SubscriberConfig,
        registry: Arc<SubscriberRegistry>,
    ) -> Result<Self, String> {
        let listener = bind_listener(&config)?;
        let local_addr = listener
            .local_addr()
            .map_err(|error| format!("read subscriber listener address: {error}"))?;
        let stop = Arc::new(AtomicBool::new(false));
        let owner = SubscriberAcceptOwner {
            listener,
            config,
            registry: Arc::clone(&registry),
            stop: Arc::clone(&stop),
            workers: Vec::new(),
        };
        let thread = thread::Builder::new()
            .name("alert-proxy-subscriber-accept".to_string())
            .spawn(move || owner.run())
            .map_err(|error| format!("spawn subscriber accept owner: {error}"))?;
        Ok(Self {
            local_addr,
            stop,
            registry,
            thread: Some(thread),
        })
    }

    pub(crate) fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub(crate) fn shutdown(&mut self) -> Result<(), String> {
        self.stop.store(true, Ordering::Release);
        self.registry.close_all();
        self.thread
            .take()
            .map(|thread| {
                thread
                    .join()
                    .map_err(|_| "subscriber accept owner panicked".to_string())
            })
            .unwrap_or(Ok(()))
    }
}

impl Drop for SubscriberServer {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

struct SubscriberAcceptOwner {
    listener: TcpListener,
    config: SubscriberConfig,
    registry: Arc<SubscriberRegistry>,
    stop: Arc<AtomicBool>,
    workers: Vec<JoinHandle<()>>,
}

impl SubscriberAcceptOwner {
    fn run(mut self) {
        while !self.stop.load(Ordering::Acquire) {
            self.reap_finished();
            match self.listener.accept() {
                Ok((stream, _)) => self.accept(stream),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(self.config.accept_poll_interval());
                }
                Err(error) => {
                    ProxyDiagnostics::runtime_failed("subscriber accept", &error);
                    thread::sleep(self.config.accept_poll_interval());
                }
            }
        }
        self.registry.close_all();
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }

    fn accept(&mut self, stream: TcpStream) {
        if self.workers.len() >= self.config.connection_limit as usize {
            return;
        }
        let session = match SubscriberSession::new(
            stream,
            self.config.clone(),
            Arc::clone(&self.registry),
            Arc::clone(&self.stop),
        ) {
            Ok(session) => session,
            Err(error) => {
                ProxyDiagnostics::connection_failed("subscriber", &error);
                return;
            }
        };
        match thread::Builder::new()
            .name("alert-proxy-subscriber-session".to_string())
            .stack_size(self.config.worker_thread_stack_bytes)
            .spawn(move || {
                if let Err(error) = session.run() {
                    ProxyDiagnostics::connection_failed("subscriber", &error);
                }
            }) {
            Ok(worker) => self.workers.push(worker),
            Err(error) => ProxyDiagnostics::runtime_failed("subscriber worker spawn", &error),
        }
    }

    fn reap_finished(&mut self) {
        let mut index = 0;
        while index < self.workers.len() {
            if self.workers[index].is_finished() {
                let worker = self.workers.swap_remove(index);
                let _ = worker.join();
            } else {
                index += 1;
            }
        }
    }
}

fn bind_listener(config: &SubscriberConfig) -> Result<TcpListener, String> {
    let domain = if config.listen_addr.is_ipv4() {
        Domain::IPV4
    } else {
        Domain::IPV6
    };
    let socket = Socket::new(domain, Type::STREAM, Some(Protocol::TCP))
        .map_err(|error| format!("create subscriber listener: {error}"))?;
    socket
        .set_reuse_address(true)
        .map_err(|error| format!("set subscriber SO_REUSEADDR: {error}"))?;
    socket
        .bind(&config.listen_addr.into())
        .map_err(|error| format!("bind subscriber listener {}: {error}", config.listen_addr))?;
    socket
        .listen(config.listen_backlog)
        .map_err(|error| format!("listen subscriber socket: {error}"))?;
    socket
        .set_nonblocking(true)
        .map_err(|error| format!("set subscriber listener nonblocking: {error}"))?;
    Ok(socket.into())
}
