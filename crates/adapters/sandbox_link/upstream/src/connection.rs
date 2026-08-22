use std::fmt;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Instant;

use gateway_ingest_runtime::{GatewayConnection, GatewayIngestRuntime};
use sandbox_upstream_contract::{
    ForwardedSbFrame, Frame, FrameCode, FrameDecoder as UpstreamFrameDecoder,
};
use sandbox_vsock_contract::{
    FrameCode as SbFrameCode, FrameDecoder as SbFrameDecoder, ObservationBatchCodec,
};

use crate::config::UpstreamServerConfig;
use crate::status::ServerMetrics;

pub(super) struct ConnectionWorker {
    stream: TcpStream,
    connection: Option<GatewayConnection>,
    runtime: GatewayIngestRuntime,
    metrics: Arc<ServerMetrics>,
    read_buffer: Vec<u8>,
    upstream_decoder: UpstreamFrameDecoder,
    batch_codec: ObservationBatchCodec,
    welcomed: bool,
    connection_idle_timeout: std::time::Duration,
}

impl ConnectionWorker {
    pub(super) fn new(
        stream: TcpStream,
        runtime: GatewayIngestRuntime,
        metrics: Arc<ServerMetrics>,
        config: &UpstreamServerConfig,
    ) -> Result<Self, ConnectionError> {
        stream
            .set_nodelay(true)
            .map_err(|error| ConnectionError::io("set_nodelay", error))?;
        stream
            .set_read_timeout(Some(config.connection_poll_interval))
            .map_err(|error| ConnectionError::io("set_read_timeout", error))?;
        stream
            .set_write_timeout(Some(config.write_timeout))
            .map_err(|error| ConnectionError::io("set_write_timeout", error))?;
        Ok(Self {
            stream,
            connection: None,
            runtime,
            metrics,
            read_buffer: vec![0; config.read_buffer_bytes],
            upstream_decoder: UpstreamFrameDecoder::with_capacity(config.read_buffer_bytes),
            batch_codec: ObservationBatchCodec,
            welcomed: false,
            connection_idle_timeout: config.connection_idle_timeout,
        })
    }

    pub(super) fn run(mut self) {
        if self.run_inner().is_err() {
            self.metrics.connection_failure();
        }
    }

    fn run_inner(&mut self) -> Result<(), ConnectionError> {
        let mut last_activity = Instant::now();
        while !self.runtime.is_shutdown_requested() {
            let read = match self.stream.read(&mut self.read_buffer) {
                Ok(0) => return Ok(()),
                Ok(read) => {
                    last_activity = Instant::now();
                    read
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    if last_activity.elapsed() >= self.connection_idle_timeout {
                        return Ok(());
                    }
                    continue;
                }
                Err(error) => return Err(ConnectionError::io("read", error)),
            };
            self.upstream_decoder.push(&self.read_buffer[..read]);
            while let Some(frame) = self
                .upstream_decoder
                .next_frame()
                .map_err(|error| ConnectionError::wire("upstream_decode", error))?
            {
                self.handle_frame(frame)?;
            }
        }
        Ok(())
    }

    fn handle_frame(&mut self, frame: Frame) -> Result<(), ConnectionError> {
        if !self.welcomed {
            if frame.code != FrameCode::GatewayHello || !frame.payload.is_empty() {
                return Err(ConnectionError::protocol(
                    "first frame must be an empty GatewayHello",
                ));
            }
            let connection = self
                .runtime
                .try_open()
                .map_err(|error| ConnectionError::protocol(error.to_string()))?;
            let gateway_id = connection.gateway_id();
            self.write_frame(Frame::numeric_id(FrameCode::GatewayWelcome, gateway_id))?;
            self.connection = Some(connection);
            self.welcomed = true;
            return Ok(());
        }

        match frame.code {
            FrameCode::Heartbeat if frame.payload.is_empty() => {
                self.connection()?.record_heartbeat();
                Ok(())
            }
            FrameCode::ForwardedSbFrame => self.handle_forwarded(frame.payload),
            FrameCode::Heartbeat => {
                Err(ConnectionError::protocol("Heartbeat payload must be empty"))
            }
            FrameCode::GatewayHello | FrameCode::GatewayWelcome => Err(ConnectionError::protocol(
                "unexpected handshake frame after GatewayWelcome",
            )),
        }
    }

    fn handle_forwarded(&mut self, payload: Vec<u8>) -> Result<(), ConnectionError> {
        let forwarded = ForwardedSbFrame::decode(&payload)
            .map_err(|error| ConnectionError::wire("forwarded_sb", error))?;
        let mut decoder = SbFrameDecoder::with_capacity(forwarded.frame_bytes.len());
        decoder.push(&forwarded.frame_bytes);
        let frame = decoder
            .next_frame()
            .map_err(|error| ConnectionError::wire("sb_frame", error))?
            .ok_or_else(|| ConnectionError::protocol("forwarded SB frame is incomplete"))?;
        if decoder
            .next_frame()
            .map_err(|error| ConnectionError::wire("sb_frame", error))?
            .is_some()
            || frame
                .encode()
                .map_err(|error| ConnectionError::wire("sb_frame", error))?
                != forwarded.frame_bytes
        {
            return Err(ConnectionError::protocol(
                "ForwardedSbFrame must contain exactly one complete SB frame",
            ));
        }
        if frame.code != SbFrameCode::ObservationBatch {
            return Err(ConnectionError::protocol(
                "ForwardedSbFrame must contain an ObservationBatch",
            ));
        }
        let batch = self
            .batch_codec
            .decode(&frame.payload)
            .map_err(|error| ConnectionError::wire("observation_batch", error))?;
        self.connection()?
            .deliver(forwarded.sb_id, batch)
            .map_err(|error| ConnectionError::wire("sink_delivery", error))
    }

    fn connection(&self) -> Result<&GatewayConnection, ConnectionError> {
        self.connection
            .as_ref()
            .ok_or_else(|| ConnectionError::protocol("gateway connection is not registered"))
    }

    fn write_frame(&mut self, frame: Frame) -> Result<(), ConnectionError> {
        let bytes = frame
            .encode()
            .map_err(|error| ConnectionError::wire("upstream_encode", error))?;
        self.stream
            .write_all(&bytes)
            .map_err(|error| ConnectionError::io("write", error))
    }
}

#[derive(Debug)]
pub(super) struct ConnectionError {
    stage: &'static str,
    message: String,
}

impl ConnectionError {
    fn protocol(message: impl Into<String>) -> Self {
        Self {
            stage: "protocol",
            message: message.into(),
        }
    }

    fn io(stage: &'static str, error: io::Error) -> Self {
        Self {
            stage,
            message: error.to_string(),
        }
    }

    fn wire(stage: &'static str, error: impl fmt::Display) -> Self {
        Self {
            stage,
            message: error.to_string(),
        }
    }
}

impl fmt::Display for ConnectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.stage, self.message)
    }
}
