use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use alert_delivery_contract::{
    AtapCodec, AtapMessage, AtapStreamDecoder, Heartbeat, HeartbeatAck, ProducerHello,
};
use alert_forwarding::{AlertForwardingPlugin, ConnectionGeneration, ForwardingItem};

pub(super) struct AlertProxyLink {
    stream: UnixStream,
    codec: AtapCodec,
    generation: ConnectionGeneration,
    receiver: Receiver<ForwardingItem>,
    plugin: AlertForwardingPlugin,
    heartbeat_interval: Duration,
    heartbeat_ack_timeout: Duration,
    thread_stack_bytes: usize,
}

impl AlertProxyLink {
    pub(super) fn handshake(
        mut stream: UnixStream,
        codec: AtapCodec,
        generation: ConnectionGeneration,
        receiver: Receiver<ForwardingItem>,
        plugin: AlertForwardingPlugin,
        heartbeat_interval: Duration,
        heartbeat_ack_timeout: Duration,
        thread_stack_bytes: usize,
    ) -> Result<Self, String> {
        let hello = codec
            .encode(&AtapMessage::ProducerHello(ProducerHello {
                daemon_pid: std::process::id(),
            }))
            .map_err(|error| error.to_string())?;
        stream
            .write_all(&hello)
            .map_err(|error| format!("write producer handshake: {error}"))?;
        match read_one_message(&mut stream, &codec)? {
            AtapMessage::ProducerWelcome => Ok(Self {
                stream,
                codec,
                generation,
                receiver,
                plugin,
                heartbeat_interval,
                heartbeat_ack_timeout,
                thread_stack_bytes,
            }),
            AtapMessage::ProducerReject(reject) => {
                Err(format!("alert proxy rejected producer: {}", reject.code))
            }
            _ => Err("alert proxy returned an invalid producer handshake response".to_string()),
        }
    }

    pub(super) fn start(self) -> Result<(), String> {
        let reader = self
            .stream
            .try_clone()
            .map_err(|error| format!("clone alert proxy connection: {error}"))?;
        let outstanding_nonce = Arc::new(AtomicU64::new(0));
        self.spawn_reader(reader, Arc::clone(&outstanding_nonce))?;
        self.spawn_writer(outstanding_nonce)
    }

    pub(super) fn set_read_timeout(&self, timeout: Duration) -> Result<(), String> {
        self.stream
            .set_read_timeout(Some(timeout))
            .map_err(|error| format!("set alert proxy read timeout: {error}"))
    }

    fn spawn_reader(
        &self,
        stream: UnixStream,
        outstanding_nonce: Arc<AtomicU64>,
    ) -> Result<(), String> {
        let codec = self.codec.clone();
        let generation = self.generation;
        let plugin = self.plugin.clone();
        std::thread::Builder::new()
            .name("alert-proxy-reader".to_string())
            .stack_size(self.thread_stack_bytes)
            .spawn(move || {
                AlertProxyReader {
                    stream,
                    codec,
                    generation,
                    plugin,
                    outstanding_nonce,
                }
                .run();
            })
            .map(|_| ())
            .map_err(|error| format!("spawn alert proxy reader: {error}"))
    }

    fn spawn_writer(self, outstanding_nonce: Arc<AtomicU64>) -> Result<(), String> {
        std::thread::Builder::new()
            .name("alert-proxy-writer".to_string())
            .stack_size(self.thread_stack_bytes)
            .spawn(move || {
                AlertProxyWriter {
                    stream: self.stream,
                    codec: self.codec,
                    generation: self.generation,
                    receiver: self.receiver,
                    plugin: self.plugin,
                    outstanding_nonce,
                    heartbeat_interval: self.heartbeat_interval,
                    heartbeat_ack_timeout: self.heartbeat_ack_timeout,
                    next_nonce: 1,
                }
                .run();
            })
            .map(|_| ())
            .map_err(|error| format!("spawn alert proxy writer: {error}"))
    }
}

struct AlertProxyReader {
    stream: UnixStream,
    codec: AtapCodec,
    generation: ConnectionGeneration,
    plugin: AlertForwardingPlugin,
    outstanding_nonce: Arc<AtomicU64>,
}

impl AlertProxyReader {
    fn run(mut self) {
        let mut decoder = AtapStreamDecoder::with_capacity(self.codec.limits().max_frame_bytes());
        let mut buffer = [0_u8; 4_096];
        while self.plugin.is_active_generation(self.generation) {
            match self.stream.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => {
                    decoder.push(&buffer[..read]);
                    if !self.consume(&mut decoder) {
                        break;
                    }
                }
                Err(error) if is_retryable(&error) => continue,
                Err(_) => break,
            }
        }
        self.plugin.disable_if_generation(self.generation);
    }

    fn consume(&self, decoder: &mut AtapStreamDecoder) -> bool {
        loop {
            match decoder.next_message(&self.codec) {
                Ok(Some(AtapMessage::HeartbeatAck(HeartbeatAck { nonce }))) => {
                    if self
                        .outstanding_nonce
                        .compare_exchange(nonce, 0, Ordering::AcqRel, Ordering::Acquire)
                        .is_err()
                    {
                        return false;
                    }
                }
                Ok(Some(_)) => return false,
                Ok(None) => return true,
                Err(_) => return false,
            }
        }
    }
}

struct AlertProxyWriter {
    stream: UnixStream,
    codec: AtapCodec,
    generation: ConnectionGeneration,
    receiver: Receiver<ForwardingItem>,
    plugin: AlertForwardingPlugin,
    outstanding_nonce: Arc<AtomicU64>,
    heartbeat_interval: Duration,
    heartbeat_ack_timeout: Duration,
    next_nonce: u64,
}

impl AlertProxyWriter {
    fn run(mut self) {
        let Some(mut next_heartbeat) = Instant::now().checked_add(self.heartbeat_interval) else {
            self.plugin.disable_if_generation(self.generation);
            return;
        };
        let mut heartbeat_deadline = None;
        while self.plugin.is_active_generation(self.generation) {
            let now = Instant::now();
            if let Some(deadline) = heartbeat_deadline {
                if self.outstanding_nonce.load(Ordering::Acquire) == 0 {
                    heartbeat_deadline = None;
                    let Some(next) = now.checked_add(self.heartbeat_interval) else {
                        break;
                    };
                    next_heartbeat = next;
                } else if now >= deadline {
                    break;
                }
            } else if now >= next_heartbeat {
                if !self.send_heartbeat() {
                    break;
                }
                let Some(deadline) = Instant::now().checked_add(self.heartbeat_ack_timeout) else {
                    break;
                };
                heartbeat_deadline = Some(deadline);
                continue;
            }
            let wake_at = heartbeat_deadline.unwrap_or(next_heartbeat);
            let wait = wake_at.saturating_duration_since(Instant::now());
            match self.receiver.recv_timeout(wait) {
                Ok(item) if item.generation() == self.generation => {
                    let frame = match self
                        .codec
                        .encode(&AtapMessage::ForwardAlert(item.into_alert()))
                    {
                        Ok(frame) => frame,
                        Err(_) => {
                            self.plugin.record_delivery_drop();
                            continue;
                        }
                    };
                    if self.stream.write_all(&frame).is_err() {
                        break;
                    }
                }
                Ok(_) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            }
        }
        self.plugin.disable_if_generation(self.generation);
    }

    fn send_heartbeat(&mut self) -> bool {
        let nonce = self.next_nonce;
        self.next_nonce = self.next_nonce.wrapping_add(1).max(1);
        self.outstanding_nonce.store(nonce, Ordering::Release);
        self.write_message(&AtapMessage::Heartbeat(Heartbeat { nonce }))
            .is_ok()
    }

    fn write_message(&mut self, message: &AtapMessage) -> Result<(), ()> {
        let frame = self.codec.encode(message).map_err(|_| ())?;
        self.stream.write_all(&frame).map_err(|_| ())
    }
}

fn read_one_message(stream: &mut UnixStream, codec: &AtapCodec) -> Result<AtapMessage, String> {
    let mut decoder = AtapStreamDecoder::with_capacity(codec.limits().max_frame_bytes());
    let mut buffer = [0_u8; 1_024];
    loop {
        let read = stream
            .read(&mut buffer)
            .map_err(|error| format!("read producer handshake: {error}"))?;
        if read == 0 {
            return Err("alert proxy closed during producer handshake".to_string());
        }
        decoder.push(&buffer[..read]);
        if let Some(message) = decoder
            .next_message(codec)
            .map_err(|error| error.to_string())?
        {
            return Ok(message);
        }
    }
}

fn is_retryable(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted
    )
}
