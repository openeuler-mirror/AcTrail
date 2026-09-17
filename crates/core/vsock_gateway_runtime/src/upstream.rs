use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};
use std::thread::{self, JoinHandle};
use std::time::Instant;

use sandbox_upstream_contract::{Frame, FrameCode, FrameDecoder, GatewayWelcome};

use crate::GatewayConfig;

pub(super) struct UpstreamLink {
    sender: SyncSender<ForwardItem>,
    stop: Arc<AtomicBool>,
    gateway_id: Arc<AtomicU32>,
    workload_cgroup_observations: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl UpstreamLink {
    pub(super) fn start(config: &GatewayConfig) -> io::Result<Self> {
        let (stream, welcome) = connect_registered(config)?;
        let (sender, receiver) = mpsc::sync_channel(config.outbound_queue_capacity);
        let stop = Arc::new(AtomicBool::new(false));
        let gateway_id_state = Arc::new(AtomicU32::new(welcome.gateway_id));
        let workload_state = Arc::new(AtomicBool::new(welcome.workload_cgroup_observations));
        let thread_stop = Arc::clone(&stop);
        let thread_gateway_id = Arc::clone(&gateway_id_state);
        let thread_workload_state = Arc::clone(&workload_state);
        let thread_config = config.clone();
        let handle = thread::Builder::new()
            .name("actrail-gateway-upstream".to_string())
            .stack_size(config.connection_thread_stack_bytes)
            .spawn(move || {
                UpstreamWorker::new(
                    thread_config,
                    receiver,
                    thread_stop,
                    thread_gateway_id,
                    thread_workload_state,
                )
                .run(stream);
            })?;
        Ok(Self {
            sender,
            stop,
            gateway_id: gateway_id_state,
            workload_cgroup_observations: workload_state,
            thread: Some(handle),
        })
    }

    pub(super) fn sender(&self) -> UpstreamSender {
        UpstreamSender {
            inner: self.sender.clone(),
            workload_cgroup_observations: Arc::clone(&self.workload_cgroup_observations),
        }
    }

    pub(super) fn gateway_id(&self) -> u32 {
        self.gateway_id.load(Ordering::Acquire)
    }

    pub(super) fn shutdown(&mut self) -> io::Result<()> {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.thread.take() {
            handle.thread().unpark();
            handle
                .join()
                .map_err(|_| io::Error::other("upstream worker panicked"))?;
        }
        Ok(())
    }
}

impl Drop for UpstreamLink {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

#[derive(Clone)]
pub(super) struct SessionForwardQuota {
    state: Arc<SessionForwardState>,
}

struct SessionForwardState {
    active: AtomicBool,
    pending: AtomicUsize,
    limit: usize,
}

impl SessionForwardQuota {
    pub(super) fn new(limit: usize) -> Self {
        Self {
            state: Arc::new(SessionForwardState {
                active: AtomicBool::new(true),
                pending: AtomicUsize::new(0),
                limit,
            }),
        }
    }

    pub(super) fn close(&self) {
        self.state.active.store(false, Ordering::Release);
    }

    fn try_reserve(&self) -> io::Result<SessionForwardPermit> {
        if !self.state.active.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "SB session is closed",
            ));
        }
        self.state
            .pending
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |pending| {
                (pending < self.state.limit).then_some(pending + 1)
            })
            .map_err(|_| {
                io::Error::new(io::ErrorKind::WouldBlock, "per-SB forward quota is full")
            })?;
        if !self.state.active.load(Ordering::Acquire) {
            self.state.pending.fetch_sub(1, Ordering::AcqRel);
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "SB session closed while reserving forward quota",
            ));
        }
        Ok(SessionForwardPermit {
            state: Arc::clone(&self.state),
        })
    }
}

struct SessionForwardPermit {
    state: Arc<SessionForwardState>,
}

impl SessionForwardPermit {
    fn is_active(&self) -> bool {
        self.state.active.load(Ordering::Acquire)
    }
}

impl Drop for SessionForwardPermit {
    fn drop(&mut self) {
        self.state.pending.fetch_sub(1, Ordering::AcqRel);
    }
}

struct ForwardItem {
    bytes: Vec<u8>,
    permit: SessionForwardPermit,
}

#[derive(Clone)]
pub(super) struct UpstreamSender {
    inner: SyncSender<ForwardItem>,
    workload_cgroup_observations: Arc<AtomicBool>,
}

impl UpstreamSender {
    pub(super) fn supports_workload_cgroup_observations(&self) -> bool {
        self.workload_cgroup_observations.load(Ordering::Acquire)
    }

    pub(super) fn try_send(&self, bytes: Vec<u8>, quota: &SessionForwardQuota) -> io::Result<()> {
        let item = ForwardItem {
            bytes,
            permit: quota.try_reserve()?,
        };
        match self.inner.try_send(item) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "gateway upstream queue is full",
            )),
            Err(TrySendError::Disconnected(_)) => Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "gateway upstream worker is closed",
            )),
        }
    }
}

struct UpstreamWorker {
    config: GatewayConfig,
    receiver: Receiver<ForwardItem>,
    stop: Arc<AtomicBool>,
    gateway_id: Arc<AtomicU32>,
    workload_cgroup_observations: Arc<AtomicBool>,
}

impl UpstreamWorker {
    fn new(
        config: GatewayConfig,
        receiver: Receiver<ForwardItem>,
        stop: Arc<AtomicBool>,
        gateway_id: Arc<AtomicU32>,
        workload_cgroup_observations: Arc<AtomicBool>,
    ) -> Self {
        Self {
            config,
            receiver,
            stop,
            gateway_id,
            workload_cgroup_observations,
        }
    }

    fn run(&self, mut stream: TcpStream) {
        let _id_lifetime = GatewayIdLifetime {
            gateway_id: Arc::clone(&self.gateway_id),
            workload_cgroup_observations: Arc::clone(&self.workload_cgroup_observations),
        };
        let mut pending = None;
        let mut last_heartbeat = Instant::now();
        while !self.stop.load(Ordering::Acquire) {
            let item = pending.take().or_else(|| match self.receiver.try_recv() {
                Ok(bytes) => Some(bytes),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => None,
            });
            let write_result = if let Some(item) = item {
                if !item.permit.is_active() {
                    continue;
                }
                let result = stream.write_all(&item.bytes);
                if result.is_err() {
                    pending = Some(item);
                }
                result
            } else if last_heartbeat.elapsed() >= self.config.upstream_heartbeat_interval {
                last_heartbeat = Instant::now();
                write_frame(
                    &mut stream,
                    &Frame::new(FrameCode::Heartbeat, Vec::new()).expect("fixed heartbeat"),
                )
            } else {
                thread::park_timeout(self.config.accept_poll_interval);
                continue;
            };
            if write_result.is_err() {
                let previously_supported =
                    self.workload_cgroup_observations.load(Ordering::Acquire);
                self.gateway_id.store(0, Ordering::Release);
                self.workload_cgroup_observations
                    .store(false, Ordering::Release);
                match self.reconnect() {
                    Some((new_stream, welcome)) => {
                        stream = new_stream;
                        if previously_supported && !welcome.workload_cgroup_observations {
                            pending = None;
                            while self.receiver.try_recv().is_ok() {}
                        }
                        self.gateway_id.store(welcome.gateway_id, Ordering::Release);
                        self.workload_cgroup_observations
                            .store(welcome.workload_cgroup_observations, Ordering::Release);
                        last_heartbeat = Instant::now();
                    }
                    None => return,
                }
            }
        }
    }

    fn reconnect(&self) -> Option<(TcpStream, GatewayWelcome)> {
        while !self.stop.load(Ordering::Acquire) {
            match connect_registered(&self.config) {
                Ok(registered) => return Some(registered),
                Err(_) => thread::park_timeout(self.config.reconnect_interval),
            }
        }
        None
    }
}

struct GatewayIdLifetime {
    gateway_id: Arc<AtomicU32>,
    workload_cgroup_observations: Arc<AtomicBool>,
}

impl Drop for GatewayIdLifetime {
    fn drop(&mut self) {
        self.gateway_id.store(0, Ordering::Release);
        self.workload_cgroup_observations
            .store(false, Ordering::Release);
    }
}

fn connect_registered(config: &GatewayConfig) -> io::Result<(TcpStream, GatewayWelcome)> {
    connect_registered_with_capabilities(config, true)
        .or_else(|_| connect_registered_with_capabilities(config, false))
}

fn connect_registered_with_capabilities(
    config: &GatewayConfig,
    request_workload_cgroup_observations: bool,
) -> io::Result<(TcpStream, GatewayWelcome)> {
    let mut stream = TcpStream::connect_timeout(&config.daemon_address, config.io_timeout)?;
    stream.set_read_timeout(Some(config.io_timeout))?;
    stream.set_write_timeout(Some(config.io_timeout))?;
    stream.set_nodelay(true)?;
    write_frame(
        &mut stream,
        &Frame::gateway_hello(request_workload_cgroup_observations),
    )?;
    let welcome = read_frame(&mut stream)?;
    if welcome.code != FrameCode::GatewayWelcome {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "daemon did not return GatewayWelcome",
        ));
    }
    let welcome = welcome
        .decode_gateway_welcome()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if !request_workload_cgroup_observations && welcome.workload_cgroup_observations {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "daemon advertised an unrequested gateway capability",
        ));
    }
    Ok((stream, welcome))
}

fn write_frame(stream: &mut TcpStream, frame: &Frame) -> io::Result<()> {
    let bytes = frame
        .encode()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    stream.write_all(&bytes)
}

fn read_frame(stream: &mut TcpStream) -> io::Result<Frame> {
    let mut decoder = FrameDecoder::with_capacity(1024);
    let mut buffer = [0_u8; 1024];
    loop {
        let count = stream.read(&mut buffer)?;
        if count == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "daemon closed during gateway handshake",
            ));
        }
        decoder.push(&buffer[..count]);
        if let Some(frame) = decoder
            .next_frame()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?
        {
            return Ok(frame);
        }
    }
}
