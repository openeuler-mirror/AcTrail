use std::io::{self, Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Instant;

use sandbox_upstream_contract::{
    ForwardedSbFrame, Frame as UpstreamFrame, FrameCode as UpstreamCode,
};
use sandbox_vsock_contract::{Frame, FrameCode, FrameDecoder};
use sandbox_vsock_transport::{VsockConnection, VsockListener};

use crate::GatewayConfig;
use crate::session::SessionRegistry;
use crate::upstream::{SessionForwardQuota, UpstreamLink, UpstreamSender};

pub struct GatewayRuntime {
    config: GatewayConfig,
    stop: Arc<AtomicBool>,
    sessions: Arc<SessionRegistry>,
    forwarded_frames: Arc<AtomicU64>,
    upstream: UpstreamLink,
    accept_thread: Option<JoinHandle<()>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GatewaySnapshot {
    pub gateway_id: u32,
    pub active_sb_connections: usize,
    pub forwarded_frames: u64,
}

impl GatewayRuntime {
    pub fn start(config: GatewayConfig) -> io::Result<Self> {
        config.validate()?;
        let upstream = UpstreamLink::start(&config)?;
        let listener = match VsockListener::bind(&config.listener) {
            Ok(listener) => listener,
            Err(error) => {
                let mut upstream = upstream;
                let _ = upstream.shutdown();
                return Err(error);
            }
        };
        listener.set_nonblocking(true)?;
        let stop = Arc::new(AtomicBool::new(false));
        let sessions = Arc::new(SessionRegistry::new(config.max_sb_connections));
        let forwarded_frames = Arc::new(AtomicU64::new(0));
        let thread_stop = Arc::clone(&stop);
        let thread_sessions = Arc::clone(&sessions);
        let thread_forwarded = Arc::clone(&forwarded_frames);
        let thread_sender = upstream.sender();
        let thread_config = config.clone();
        let accept_thread = thread::Builder::new()
            .name("actrail-gateway-vsock-accept".to_string())
            .stack_size(config.connection_thread_stack_bytes)
            .spawn(move || {
                AcceptLoop::new(
                    thread_config,
                    listener,
                    thread_sender,
                    thread_sessions,
                    thread_forwarded,
                    thread_stop,
                )
                .run();
            })?;
        Ok(Self {
            config,
            stop,
            sessions,
            forwarded_frames,
            upstream,
            accept_thread: Some(accept_thread),
        })
    }

    pub fn snapshot(&self) -> GatewaySnapshot {
        GatewaySnapshot {
            gateway_id: self.upstream.gateway_id(),
            active_sb_connections: self.sessions.active_count(),
            forwarded_frames: self.forwarded_frames.load(Ordering::Acquire),
        }
    }

    pub fn shutdown(&mut self) -> io::Result<()> {
        self.stop.store(true, Ordering::Release);
        let accept_result = self.accept_thread.take().map_or(Ok(()), |handle| {
            handle.thread().unpark();
            handle
                .join()
                .map_err(|_| io::Error::other("gateway accept thread panicked"))
        });
        let upstream_result = self.upstream.shutdown();
        accept_result.and(upstream_result)
    }

    pub fn config(&self) -> &GatewayConfig {
        &self.config
    }
}

impl Drop for GatewayRuntime {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

struct AcceptLoop {
    config: GatewayConfig,
    listener: VsockListener,
    upstream: UpstreamSender,
    sessions: Arc<SessionRegistry>,
    forwarded_frames: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
}

impl AcceptLoop {
    fn new(
        config: GatewayConfig,
        listener: VsockListener,
        upstream: UpstreamSender,
        sessions: Arc<SessionRegistry>,
        forwarded_frames: Arc<AtomicU64>,
        stop: Arc<AtomicBool>,
    ) -> Self {
        Self {
            config,
            listener,
            upstream,
            sessions,
            forwarded_frames,
            stop,
        }
    }

    fn run(&self) {
        let mut workers: Vec<JoinHandle<()>> = Vec::new();
        while !self.stop.load(Ordering::Acquire) {
            Self::reap_finished(&mut workers);
            match self.listener.accept() {
                Ok(connection) => self.spawn_connection(connection, &mut workers),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::park_timeout(self.config.accept_poll_interval);
                }
                Err(_) => thread::park_timeout(self.config.accept_poll_interval),
            }
        }
        for worker in workers {
            let _ = worker.join();
        }
    }

    fn reap_finished(workers: &mut Vec<JoinHandle<()>>) {
        let mut index = 0;
        while index < workers.len() {
            if workers[index].is_finished() {
                let worker = workers.swap_remove(index);
                let _ = worker.join();
            } else {
                index += 1;
            }
        }
    }

    fn spawn_connection(&self, connection: VsockConnection, workers: &mut Vec<JoinHandle<()>>) {
        if workers.len() >= self.config.max_sb_connections {
            return;
        }
        let worker = SbConnectionWorker {
            connection,
            upstream: self.upstream.clone(),
            sessions: Arc::clone(&self.sessions),
            forwarded_frames: Arc::clone(&self.forwarded_frames),
            stop: Arc::clone(&self.stop),
            config: self.config.clone(),
        };
        match thread::Builder::new()
            .name("actrail-gateway-sb-pending".to_string())
            .stack_size(self.config.connection_thread_stack_bytes)
            .spawn(move || worker.run())
        {
            Ok(handle) => workers.push(handle),
            Err(_) => {}
        }
    }
}

struct SbConnectionWorker {
    connection: VsockConnection,
    upstream: UpstreamSender,
    sessions: Arc<SessionRegistry>,
    forwarded_frames: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    config: GatewayConfig,
}

impl SbConnectionWorker {
    fn run(mut self) {
        if self
            .connection
            .set_timeouts(self.config.io_timeout)
            .is_err()
        {
            return;
        }
        let mut decoder = FrameDecoder::with_capacity(4096);
        let Some(hello) = self.read_next_frame(&mut decoder, self.config.sb_peer_idle_timeout)
        else {
            return;
        };
        if hello.code != FrameCode::SbHello || !hello.payload.is_empty() {
            return;
        }
        let Ok(sb_id) = self.sessions.allocate() else {
            return;
        };
        let forward_quota = SessionForwardQuota::new(self.config.per_sb_forward_quota);
        let _guard = SessionRelease {
            id: sb_id,
            sessions: Arc::clone(&self.sessions),
            forward_quota: forward_quota.clone(),
        };
        let welcome = Frame::numeric_id(FrameCode::SbWelcome, sb_id);
        let Ok(welcome_bytes) = welcome.encode() else {
            return;
        };
        if self.connection.write_all(&welcome_bytes).is_err() {
            return;
        }
        let mut last_activity = Instant::now();
        while !self.stop.load(Ordering::Acquire) {
            match self.read_frame_once(&mut decoder) {
                Ok(Some(frame)) => {
                    last_activity = Instant::now();
                    match frame.code {
                        FrameCode::Heartbeat if frame.payload.is_empty() => {}
                        FrameCode::ObservationBatch => {
                            if self.forward(sb_id, &forward_quota, frame).is_err() {
                                return;
                            }
                        }
                        _ => return,
                    }
                }
                Ok(None) => {
                    if last_activity.elapsed() >= self.config.sb_peer_idle_timeout {
                        return;
                    }
                }
                Err(_) => return,
            }
        }
    }

    fn forward(
        &self,
        sb_id: u32,
        forward_quota: &SessionForwardQuota,
        frame: Frame,
    ) -> io::Result<()> {
        let inner = frame
            .encode()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let payload = ForwardedSbFrame::new(sb_id, inner)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?
            .encode();
        let bytes = UpstreamFrame::new(UpstreamCode::ForwardedSbFrame, payload)
            .and_then(|frame| frame.encode())
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        self.upstream.try_send(bytes, forward_quota)?;
        self.forwarded_frames.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn read_next_frame(
        &mut self,
        decoder: &mut FrameDecoder,
        deadline: std::time::Duration,
    ) -> Option<Frame> {
        let started = Instant::now();
        while started.elapsed() < deadline && !self.stop.load(Ordering::Acquire) {
            match self.read_frame_once(decoder) {
                Ok(Some(frame)) => return Some(frame),
                Ok(None) => continue,
                Err(_) => return None,
            }
        }
        None
    }

    fn read_frame_once(&mut self, decoder: &mut FrameDecoder) -> io::Result<Option<Frame>> {
        if let Some(frame) = decoder
            .next_frame()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?
        {
            return Ok(Some(frame));
        }
        let mut buffer = [0_u8; 8192];
        match self.connection.read(&mut buffer) {
            Ok(0) => Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "SB closed VSOCK connection",
            )),
            Ok(count) => {
                decoder.push(&buffer[..count]);
                decoder
                    .next_frame()
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }
}

struct SessionRelease {
    id: u32,
    sessions: Arc<SessionRegistry>,
    forward_quota: SessionForwardQuota,
}

impl Drop for SessionRelease {
    fn drop(&mut self) {
        self.forward_quota.close();
        self.sessions.release(self.id);
    }
}
