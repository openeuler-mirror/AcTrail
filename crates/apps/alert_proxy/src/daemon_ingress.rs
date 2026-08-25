use std::fs::{self, Permissions};
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Instant;

use alert_delivery_contract::{
    AtapCodec, AtapLimits, AtapMessage, AtapStreamDecoder, HeartbeatAck, ProducerHello,
    ProducerReject,
};

use crate::broadcaster::AlertBroadcaster;
use crate::diagnostics::ProxyDiagnostics;
use crate::startup::DaemonIngressConfig;

pub(crate) struct DaemonIngressServer {
    socket_path: PathBuf,
    stop: Arc<AtomicBool>,
    active_producer: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl DaemonIngressServer {
    pub(crate) fn start(
        config: DaemonIngressConfig,
        broadcaster: Arc<AlertBroadcaster>,
        ready: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        let listener = bind_listener(&config)?;
        let socket_path = config.socket_path.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let active_producer = Arc::new(AtomicBool::new(false));
        let owner = DaemonAcceptOwner {
            listener,
            config,
            broadcaster,
            ready,
            stop: Arc::clone(&stop),
            active_producer: Arc::clone(&active_producer),
            workers: Vec::new(),
        };
        let thread = thread::Builder::new()
            .name("alert-proxy-daemon-accept".to_string())
            .spawn(move || owner.run())
            .map_err(|error| format!("spawn daemon ingress accept owner: {error}"))?;
        Ok(Self {
            socket_path,
            stop,
            active_producer,
            thread: Some(thread),
        })
    }

    pub(crate) fn shutdown(&mut self) -> Result<(), String> {
        self.stop.store(true, Ordering::Release);
        let result = self
            .thread
            .take()
            .map(|thread| {
                thread
                    .join()
                    .map_err(|_| "daemon ingress accept owner panicked".to_string())
            })
            .unwrap_or(Ok(()));
        self.active_producer.store(false, Ordering::Release);
        match fs::remove_file(&self.socket_path) {
            Ok(()) => result,
            Err(error) if error.kind() == io::ErrorKind::NotFound => result,
            Err(error) => Err(format!(
                "remove daemon ingress socket {}: {error}",
                self.socket_path.display()
            )),
        }
    }
}

impl Drop for DaemonIngressServer {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

struct DaemonAcceptOwner {
    listener: UnixListener,
    config: DaemonIngressConfig,
    broadcaster: Arc<AlertBroadcaster>,
    ready: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    active_producer: Arc<AtomicBool>,
    workers: Vec<JoinHandle<()>>,
}

impl DaemonAcceptOwner {
    fn run(mut self) {
        while !self.stop.load(Ordering::Acquire) {
            self.reap_finished();
            match self.listener.accept() {
                Ok((stream, _)) => self.spawn_worker(stream),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(self.config.accept_poll_interval());
                }
                Err(error) => {
                    ProxyDiagnostics::runtime_failed("daemon ingress accept", &error);
                    thread::sleep(self.config.accept_poll_interval());
                }
            }
        }
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }

    fn spawn_worker(&mut self, stream: UnixStream) {
        if self.workers.len() >= self.config.connection_limit as usize {
            return;
        }
        let worker = match DaemonConnection::new(
            stream,
            self.config.clone(),
            Arc::clone(&self.broadcaster),
            Arc::clone(&self.ready),
            Arc::clone(&self.stop),
            Arc::clone(&self.active_producer),
        ) {
            Ok(worker) => worker,
            Err(error) => {
                ProxyDiagnostics::connection_failed("daemon ingress", &error);
                return;
            }
        };
        match thread::Builder::new()
            .name("alert-proxy-daemon-session".to_string())
            .stack_size(self.config.worker_thread_stack_bytes)
            .spawn(move || {
                if let Err(error) = worker.run() {
                    ProxyDiagnostics::connection_failed("daemon ingress", &error);
                }
            }) {
            Ok(worker) => self.workers.push(worker),
            Err(error) => ProxyDiagnostics::runtime_failed("daemon worker spawn", &error),
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

struct DaemonConnection {
    stream: UnixStream,
    peer: PeerCredentials,
    config: DaemonIngressConfig,
    codec: AtapCodec,
    decoder: AtapStreamDecoder,
    broadcaster: Arc<AlertBroadcaster>,
    ready: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    active_producer: Arc<AtomicBool>,
    claimed: bool,
}

impl DaemonConnection {
    fn new(
        stream: UnixStream,
        config: DaemonIngressConfig,
        broadcaster: Arc<AlertBroadcaster>,
        ready: Arc<AtomicBool>,
        stop: Arc<AtomicBool>,
        active_producer: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        stream
            .set_read_timeout(Some(config.io_poll_interval()))
            .map_err(|error| format!("set daemon read timeout: {error}"))?;
        stream
            .set_write_timeout(Some(config.io_poll_interval()))
            .map_err(|error| format!("set daemon write timeout: {error}"))?;
        let peer = PeerCredentials::read(&stream)?;
        let limits = AtapLimits::new(
            config.max_frame_bytes,
            config.max_trace_id_bytes,
            config.max_category_bytes,
            config.max_description_bytes,
            config.max_extras_bytes,
        )
        .map_err(|error| error.to_string())?;
        Ok(Self {
            stream,
            peer,
            decoder: AtapStreamDecoder::with_capacity(config.max_frame_bytes),
            codec: AtapCodec::new(limits),
            config,
            broadcaster,
            ready,
            stop,
            active_producer,
            claimed: false,
        })
    }

    fn run(mut self) -> Result<(), String> {
        self.run_inner()
    }

    fn run_inner(&mut self) -> Result<(), String> {
        let first = self.read_next(Instant::now())?;
        let AtapMessage::ProducerHello(hello) = first else {
            return self.reject("producer_hello_required");
        };
        self.validate_peer(hello)?;
        while !self.ready.load(Ordering::Acquire) {
            if self.stop.load(Ordering::Acquire) {
                return self.reject("proxy_stopping");
            }
            thread::sleep(self.config.io_poll_interval());
        }
        if self
            .active_producer
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return self.reject("producer_slot_busy");
        }
        self.claimed = true;
        self.write_message(&AtapMessage::ProducerWelcome)?;
        let mut last_activity = Instant::now();
        while !self.stop.load(Ordering::Acquire) {
            match self.read_available()? {
                Some(message) => {
                    last_activity = Instant::now();
                    self.handle_message(message)?;
                }
                None if last_activity.elapsed() >= self.config.producer_idle_timeout() => {
                    return Ok(());
                }
                None => {}
            }
        }
        Ok(())
    }

    fn validate_peer(&self, hello: ProducerHello) -> Result<(), String> {
        if hello.daemon_pid != self.peer.pid {
            return Err("ProducerHello PID does not match SO_PEERCRED".to_string());
        }
        if !self.config.allowed_uids.contains(&self.peer.uid) {
            return Err("producer uid is not allowed".to_string());
        }
        if !self.config.allowed_gids.contains(&self.peer.gid) {
            return Err("producer gid is not allowed".to_string());
        }
        Ok(())
    }

    fn handle_message(&mut self, message: AtapMessage) -> Result<(), String> {
        match message {
            AtapMessage::ForwardAlert(alert) => {
                self.broadcaster.try_publish(alert);
                Ok(())
            }
            AtapMessage::Heartbeat(heartbeat) => {
                self.write_message(&AtapMessage::HeartbeatAck(HeartbeatAck {
                    nonce: heartbeat.nonce,
                }))
            }
            AtapMessage::ProducerHello(_)
            | AtapMessage::ProducerWelcome
            | AtapMessage::ProducerReject(_)
            | AtapMessage::HeartbeatAck(_) => {
                Err("unexpected ATAP message after ProducerWelcome".to_string())
            }
        }
    }

    fn read_next(&mut self, started_at: Instant) -> Result<AtapMessage, String> {
        loop {
            if let Some(message) = self.read_available()? {
                return Ok(message);
            }
            if started_at.elapsed() >= self.config.producer_idle_timeout() {
                return Err("producer handshake timed out".to_string());
            }
        }
    }

    fn read_available(&mut self) -> Result<Option<AtapMessage>, String> {
        if let Some(message) = self
            .decoder
            .next_message(&self.codec)
            .map_err(|error| error.to_string())?
        {
            return Ok(Some(message));
        }
        let mut buffer = [0_u8; 8192];
        match self.stream.read(&mut buffer) {
            Ok(0) => Err("producer closed the connection".to_string()),
            Ok(read) => {
                self.decoder.push(&buffer[..read]);
                self.decoder
                    .next_message(&self.codec)
                    .map_err(|error| error.to_string())
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                Ok(None)
            }
            Err(error) => Err(format!("read producer frame: {error}")),
        }
    }

    fn reject(&mut self, code: &str) -> Result<(), String> {
        self.write_message(&AtapMessage::ProducerReject(ProducerReject {
            code: code.to_string(),
        }))?;
        Err(code.to_string())
    }

    fn write_message(&mut self, message: &AtapMessage) -> Result<(), String> {
        let frame = self
            .codec
            .encode(message)
            .map_err(|error| error.to_string())?;
        self.stream
            .write_all(&frame)
            .map_err(|error| format!("write producer frame: {error}"))
    }
}

impl Drop for DaemonConnection {
    fn drop(&mut self) {
        if self.claimed {
            self.active_producer.store(false, Ordering::Release);
        }
    }
}

#[derive(Clone, Copy)]
struct PeerCredentials {
    pid: u32,
    uid: u32,
    gid: u32,
}

impl PeerCredentials {
    fn read(stream: &UnixStream) -> Result<Self, String> {
        let mut credentials: libc::ucred = unsafe { std::mem::zeroed() };
        let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        let result = unsafe {
            libc::getsockopt(
                stream.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                (&mut credentials as *mut libc::ucred).cast(),
                &mut length,
            )
        };
        if result < 0 {
            return Err(format!(
                "read producer peer credentials: {}",
                io::Error::last_os_error()
            ));
        }
        let pid = u32::try_from(credentials.pid)
            .map_err(|error| format!("producer pid overflow: {error}"))?;
        Ok(Self {
            pid,
            uid: credentials.uid,
            gid: credentials.gid,
        })
    }
}

fn bind_listener(config: &DaemonIngressConfig) -> Result<UnixListener, String> {
    prepare_socket_path(&config.socket_path)?;
    let listener = UnixListener::bind(&config.socket_path)
        .map_err(|error| format!("bind {}: {error}", config.socket_path.display()))?;
    let setup = (|| {
        listener
            .set_nonblocking(true)
            .map_err(|error| format!("set daemon listener nonblocking: {error}"))?;
        fs::set_permissions(
            &config.socket_path,
            Permissions::from_mode(config.socket_mode()?),
        )
        .map_err(|error| format!("set daemon socket permissions: {error}"))?;
        set_socket_owner(&config.socket_path, config.socket_uid, config.socket_gid)
    })();
    if let Err(error) = setup {
        let _ = fs::remove_file(&config.socket_path);
        return Err(error);
    }
    Ok(listener)
}

fn prepare_socket_path(path: &Path) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| "daemon ingress socket has no parent directory".to_string())?;
    fs::create_dir_all(parent).map_err(|error| {
        format!(
            "create daemon socket directory {}: {error}",
            parent.display()
        )
    })?;
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("inspect daemon socket {}: {error}", path.display())),
    };
    if !metadata.file_type().is_socket() {
        return Err(format!(
            "daemon ingress path {} exists and is not a socket",
            path.display()
        ));
    }
    match UnixStream::connect(path) {
        Ok(_) => Err(format!(
            "daemon ingress socket {} already has an active listener",
            path.display()
        )),
        Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => fs::remove_file(path)
            .map_err(|remove| format!("remove stale socket {}: {remove}", path.display())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "cannot verify daemon ingress socket {} as stale: {error}",
            path.display()
        )),
    }
}

fn set_socket_owner(path: &Path, uid: Option<u32>, gid: Option<u32>) -> Result<(), String> {
    if uid.is_none() && gid.is_none() {
        return Ok(());
    }
    let path = std::ffi::CString::new(path.as_os_str().as_encoded_bytes())
        .map_err(|_| "daemon ingress socket path contains NUL".to_string())?;
    let result = unsafe {
        libc::chown(
            path.as_ptr(),
            uid.unwrap_or(u32::MAX),
            gid.unwrap_or(u32::MAX),
        )
    };
    if result < 0 {
        Err(format!(
            "set daemon ingress socket owner: {}",
            io::Error::last_os_error()
        ))
    } else {
        Ok(())
    }
}
