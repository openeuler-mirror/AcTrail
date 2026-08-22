use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, TryRecvError};
use std::thread;
use std::time::Instant;

use sandbox_observation::{Observation, ObservationBatch};
use sandbox_vsock_contract::{Frame, FrameCode, FrameDecoder, ObservationBatchCodec};

use crate::{SandboxAgentConfig, SandboxConnection, SandboxTransport};

pub(super) struct SenderMetrics {
    pub(super) sb_id: AtomicU32,
    pub(super) sent_batches: AtomicU64,
    pub(super) reconnects: AtomicU64,
}

impl SenderMetrics {
    pub(super) fn new(sb_id: u32) -> Self {
        Self {
            sb_id: AtomicU32::new(sb_id),
            sent_batches: AtomicU64::new(0),
            reconnects: AtomicU64::new(0),
        }
    }
}

pub(super) struct ObservationSender {
    config: SandboxAgentConfig,
    transport: Arc<dyn SandboxTransport>,
    receiver: Receiver<Observation>,
    stop: Arc<AtomicBool>,
    metrics: Arc<SenderMetrics>,
    codec: ObservationBatchCodec,
    next_sequence: u64,
}

impl ObservationSender {
    pub(super) fn register(
        transport: &dyn SandboxTransport,
    ) -> io::Result<(Box<dyn SandboxConnection>, u32)> {
        let mut connection = transport.connect()?;
        write_frame(
            &mut *connection,
            &Frame::new(FrameCode::SbHello, Vec::new())
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?,
        )?;
        let welcome = read_frame(&mut *connection)?;
        if welcome.code != FrameCode::SbWelcome {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "gateway did not return SbWelcome",
            ));
        }
        let sb_id = welcome
            .decode_numeric_id()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if sb_id == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "gateway assigned reserved SB ID zero",
            ));
        }
        Ok((connection, sb_id))
    }

    pub(super) fn new(
        config: SandboxAgentConfig,
        transport: Arc<dyn SandboxTransport>,
        receiver: Receiver<Observation>,
        stop: Arc<AtomicBool>,
        metrics: Arc<SenderMetrics>,
    ) -> Self {
        Self {
            config,
            transport,
            receiver,
            stop,
            metrics,
            codec: ObservationBatchCodec,
            next_sequence: 1,
        }
    }

    pub(super) fn run(mut self, mut connection: Box<dyn SandboxConnection>) {
        let _id_lifetime = SbIdLifetime {
            metrics: Arc::clone(&self.metrics),
        };
        let mut pending: Vec<Observation> = Vec::with_capacity(self.config.batch_max_observations);
        let mut last_write = Instant::now();
        loop {
            self.fill_pending(&mut pending);
            if !pending.is_empty() {
                if self.send_batch(&mut *connection, &pending).is_err() {
                    let Some(registered) = self.reconnect() else {
                        return;
                    };
                    connection = registered;
                    continue;
                }
                pending.clear();
                self.next_sequence = self.next_sequence.wrapping_add(1).max(1);
                self.metrics.sent_batches.fetch_add(1, Ordering::Relaxed);
                last_write = Instant::now();
            } else if last_write.elapsed() >= self.config.heartbeat_interval {
                let heartbeat =
                    Frame::new(FrameCode::Heartbeat, Vec::new()).expect("fixed heartbeat");
                if write_frame(&mut *connection, &heartbeat).is_err() {
                    let Some(registered) = self.reconnect() else {
                        return;
                    };
                    connection = registered;
                }
                last_write = Instant::now();
            }
            if self.stop.load(Ordering::Acquire) {
                while pending.len() < self.config.batch_max_observations {
                    match self.receiver.try_recv() {
                        Ok(observation) => pending.push(observation),
                        Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
                    }
                }
                if !pending.is_empty() {
                    let _ = self.send_batch(&mut *connection, &pending);
                }
                return;
            }
        }
    }

    fn fill_pending(&self, pending: &mut Vec<Observation>) {
        if pending.is_empty() {
            match self.receiver.recv_timeout(self.config.heartbeat_interval) {
                Ok(observation) => pending.push(observation),
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => return,
            }
        }
        while pending.len() < self.config.batch_max_observations {
            match self.receiver.try_recv() {
                Ok(observation) => pending.push(observation),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        }
    }

    fn send_batch(
        &self,
        connection: &mut dyn SandboxConnection,
        observations: &[Observation],
    ) -> io::Result<()> {
        let batch = ObservationBatch::new(self.next_sequence, observations.to_vec());
        let payload = self
            .codec
            .encode(&batch)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let frame = Frame::new(FrameCode::ObservationBatch, payload)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        write_frame(connection, &frame)
    }

    fn reconnect(&self) -> Option<Box<dyn SandboxConnection>> {
        self.metrics.sb_id.store(0, Ordering::Release);
        while !self.stop.load(Ordering::Acquire) {
            match Self::register(&*self.transport) {
                Ok((connection, sb_id)) => {
                    self.metrics.sb_id.store(sb_id, Ordering::Release);
                    self.metrics.reconnects.fetch_add(1, Ordering::Relaxed);
                    return Some(connection);
                }
                Err(_) => thread::park_timeout(self.config.reconnect_interval),
            }
        }
        None
    }
}

struct SbIdLifetime {
    metrics: Arc<SenderMetrics>,
}

impl Drop for SbIdLifetime {
    fn drop(&mut self) {
        self.metrics.sb_id.store(0, Ordering::Release);
    }
}

fn write_frame(connection: &mut dyn SandboxConnection, frame: &Frame) -> io::Result<()> {
    let bytes = frame
        .encode()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    connection.write_all(&bytes)
}

fn read_frame(connection: &mut dyn SandboxConnection) -> io::Result<Frame> {
    let mut decoder = FrameDecoder::with_capacity(1024);
    let mut buffer = [0_u8; 1024];
    loop {
        let count = connection.read(&mut buffer)?;
        if count == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "gateway closed during SB handshake",
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
