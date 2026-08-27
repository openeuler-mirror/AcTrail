use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use alert_delivery_contract::{
    HandshakeResponse, JsonFrameCodec, JsonFrameDecoder, PingMessage, SubscribeResponse,
    SubscriberErrorResponse, SubscriberRequest,
};
use uuid::Uuid;

use crate::registry::{SubscriberHandle, SubscriberRegistry, Subscription};
use crate::startup::SubscriberConfig;

use super::activity::{HeartbeatAction, SessionActivity};
use super::auth::TokenVerifier;

pub(super) struct SubscriberSession {
    reader: TcpStream,
    writer: Arc<Mutex<TcpStream>>,
    config: SubscriberConfig,
    registry: Arc<SubscriberRegistry>,
    stop: Arc<AtomicBool>,
    codec: JsonFrameCodec,
    decoder: JsonFrameDecoder,
    verifier: TokenVerifier,
}

impl SubscriberSession {
    pub(super) fn new(
        stream: TcpStream,
        config: SubscriberConfig,
        registry: Arc<SubscriberRegistry>,
        stop: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        stream
            .set_nodelay(true)
            .map_err(|error| format!("set subscriber TCP_NODELAY: {error}"))?;
        stream
            .set_read_timeout(Some(config.io_poll_interval()))
            .map_err(|error| format!("set subscriber read timeout: {error}"))?;
        stream
            .set_write_timeout(Some(config.io_poll_interval()))
            .map_err(|error| format!("set subscriber write timeout: {error}"))?;
        let writer = stream
            .try_clone()
            .map_err(|error| format!("clone subscriber stream: {error}"))?;
        let codec = JsonFrameCodec::new(config.max_json_payload_bytes())
            .map_err(|error| error.to_string())?;
        Ok(Self {
            reader: stream,
            writer: Arc::new(Mutex::new(writer)),
            decoder: JsonFrameDecoder::with_capacity(config.max_frame_bytes),
            verifier: TokenVerifier::new(&config.allowed_tokens),
            codec,
            config,
            registry,
            stop,
        })
    }

    pub(super) fn run(mut self) -> Result<(), String> {
        let request = self.read_handshake()?;
        let SubscriberRequest::Handshake(handshake) = request else {
            return self.fail_handshake("handshake_required", "first request must be handshake");
        };
        if handshake.version != "v1" {
            return self.fail_handshake("unsupported_version", "protocol version must be v1");
        }
        if handshake.client_id.is_empty()
            || handshake.client_id.len() > self.config.max_client_id_bytes
        {
            return self.fail_handshake("invalid_client_id", "client_id is invalid");
        }
        if !self.verifier.accepts(&handshake.auth.token) {
            return self.fail_handshake("authentication_failed", "authentication failed");
        }

        let session_id = format!("sess_{}", Uuid::new_v4());
        self.write_direct(&HandshakeResponse::new(
            &session_id,
            self.config.heartbeat_interval().as_secs(),
        ))?;

        let (outbound, receiver) = mpsc::sync_channel(self.config.queue_capacity);
        let closer = self
            .reader
            .try_clone()
            .map_err(|error| format!("clone subscriber closer: {error}"))?;
        let handle = Arc::new(SubscriberHandle::new(session_id.clone(), outbound, closer));
        self.registry.register(Arc::clone(&handle))?;
        let activity = Arc::new(SessionActivity::new());
        let writer = SessionWriter {
            stream: Arc::clone(&self.writer),
            receiver,
            handle: Arc::clone(&handle),
            activity: Arc::clone(&activity),
            stop: Arc::clone(&self.stop),
            codec: self.codec,
            config: self.config.clone(),
        };
        let writer_thread = match thread::Builder::new()
            .name("alert-proxy-subscriber-writer".to_string())
            .stack_size(self.config.worker_thread_stack_bytes)
            .spawn(move || writer.run())
        {
            Ok(thread) => thread,
            Err(error) => {
                self.registry.remove(&session_id);
                handle.close();
                return Err(format!("spawn subscriber writer: {error}"));
            }
        };

        let reader_result = self.run_reader(&handle, &activity);
        self.registry.remove(&session_id);
        handle.close();
        let writer_result = writer_thread
            .join()
            .map_err(|_| "subscriber writer panicked".to_string())?;
        reader_result.and(writer_result)
    }

    fn run_reader(
        &mut self,
        handle: &Arc<SubscriberHandle>,
        activity: &Arc<SessionActivity>,
    ) -> Result<(), String> {
        while !self.stop.load(Ordering::Acquire) && !handle.is_closed() {
            let Some(request) = self.read_request()? else {
                continue;
            };
            match request {
                SubscriberRequest::Subscribe(subscribe) => {
                    activity.record_request()?;
                    if let Err(message) = self.validate_subscription(&subscribe) {
                        self.write_error(SubscriberErrorResponse::request(
                            subscribe.id,
                            "invalid_subscription",
                            message,
                        ))?;
                        return Err("invalid subscription".to_string());
                    }
                    let response =
                        SubscribeResponse::new(subscribe.id.clone(), subscribe.topics.clone());
                    let confirmation: Arc<[u8]> = self
                        .codec
                        .encode(&response)
                        .map_err(|error| error.to_string())?
                        .into();
                    handle.accept_subscription(
                        Subscription::new(subscribe.topics, subscribe.filter.severity),
                        confirmation,
                    )?;
                }
                SubscriberRequest::Pong(pong) => {
                    activity.accept_pong(pong.nonce)?;
                }
                SubscriberRequest::Handshake(_) => {
                    self.write_error(SubscriberErrorResponse::handshake(
                        "unexpected_handshake",
                        "handshake is only valid as the first request",
                    ))?;
                    return Err("unexpected handshake".to_string());
                }
            }
        }
        Ok(())
    }

    fn validate_subscription(
        &self,
        subscribe: &alert_delivery_contract::SubscribeRequest,
    ) -> Result<(), String> {
        if subscribe.id.is_empty() || subscribe.id.len() > self.config.max_request_id_bytes {
            return Err("request id is invalid".to_string());
        }
        if subscribe.topics.len() > self.config.max_topics {
            return Err("too many topics".to_string());
        }
        for topic in &subscribe.topics {
            if topic.is_empty()
                || topic.len() > self.config.max_topic_bytes
                || !topic.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b'/')
                })
            {
                return Err("subscription topic is invalid".to_string());
            }
        }
        if !subscribe.filter.tags.is_empty() {
            return Err("filter.tags must be empty in v1".to_string());
        }
        if !Subscription::validates_severities(&subscribe.filter.severity) {
            return Err("filter.severity must contain at most one of each supported value".into());
        }
        Ok(())
    }

    fn read_handshake(&mut self) -> Result<SubscriberRequest, String> {
        let started_at = Instant::now();
        loop {
            if let Some(request) = self.read_request()? {
                return Ok(request);
            }
            if self.stop.load(Ordering::Acquire) {
                return Err("proxy is stopping".to_string());
            }
            if started_at.elapsed() >= self.config.peer_idle_timeout() {
                return Err("subscriber handshake timed out".to_string());
            }
        }
    }

    fn read_request(&mut self) -> Result<Option<SubscriberRequest>, String> {
        if let Some(request) = self
            .decoder
            .next(&self.codec)
            .map_err(|error| error.to_string())?
        {
            return Ok(Some(request));
        }
        let mut buffer = [0_u8; 8192];
        match self.reader.read(&mut buffer) {
            Ok(0) => Err("subscriber closed the connection".to_string()),
            Ok(read) => {
                self.decoder.push(&buffer[..read]);
                self.decoder
                    .next(&self.codec)
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
            Err(error) => Err(format!("read subscriber frame: {error}")),
        }
    }

    fn fail_handshake<T>(&mut self, code: &str, message: &str) -> Result<T, String> {
        self.write_error(SubscriberErrorResponse::handshake(code, message))?;
        Err(message.to_string())
    }

    fn write_error(&mut self, error: SubscriberErrorResponse) -> Result<(), String> {
        let frame = self
            .codec
            .encode(&error)
            .map_err(|error| error.to_string())?;
        let mut stream = self
            .writer
            .lock()
            .map_err(|_| "subscriber writer lock is poisoned".to_string())?;
        let result = stream
            .write_all(&frame)
            .map_err(|error| format!("write subscriber frame: {error}"));
        let _ = stream.shutdown(Shutdown::Both);
        result
    }

    fn write_direct<T: serde::Serialize>(&mut self, message: &T) -> Result<(), String> {
        let frame = self
            .codec
            .encode(message)
            .map_err(|error| error.to_string())?;
        let mut stream = self
            .writer
            .lock()
            .map_err(|_| "subscriber writer lock is poisoned".to_string())?;
        stream
            .write_all(&frame)
            .map_err(|error| format!("write subscriber frame: {error}"))
    }
}

struct SessionWriter {
    stream: Arc<Mutex<TcpStream>>,
    receiver: Receiver<Arc<[u8]>>,
    handle: Arc<SubscriberHandle>,
    activity: Arc<SessionActivity>,
    stop: Arc<AtomicBool>,
    codec: JsonFrameCodec,
    config: SubscriberConfig,
}

impl SessionWriter {
    fn run(self) -> Result<(), String> {
        let result = self.run_inner();
        self.handle.close();
        result
    }

    fn run_inner(&self) -> Result<(), String> {
        while !self.stop.load(Ordering::Acquire) && !self.handle.is_closed() {
            match self.receiver.recv_timeout(self.config.io_poll_interval()) {
                Ok(frame) => self.write_frame(&frame)?,
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return Ok(()),
            }
            match self.activity.heartbeat_action(
                self.config.heartbeat_interval(),
                self.config.pong_timeout(),
                self.config.peer_idle_timeout(),
            ) {
                HeartbeatAction::None => {}
                HeartbeatAction::Close => return Ok(()),
                HeartbeatAction::Send { nonce } => {
                    let frame = self
                        .codec
                        .encode(&PingMessage::new(nonce, epoch_ms()))
                        .map_err(|error| error.to_string())?;
                    self.write_frame(&frame)?;
                }
            }
        }
        Ok(())
    }

    fn write_frame(&self, frame: &[u8]) -> Result<(), String> {
        let mut stream = self
            .stream
            .lock()
            .map_err(|_| "subscriber writer lock is poisoned".to_string())?;
        stream
            .write_all(frame)
            .map_err(|error| format!("write subscriber frame: {error}"))
    }
}

fn epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}
