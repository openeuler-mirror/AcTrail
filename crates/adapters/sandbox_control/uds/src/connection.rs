//! Nonblocking, one-command connection state owned by the server runtime.

use std::io::{Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use sandbox_control::{SandboxControlCommand, SandboxControlResponse};

use crate::{SandboxControlCodec, SandboxControlUdsError, SandboxControlUdsStage};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SandboxControlConnectionLimits {
    request_bytes: usize,
    response_bytes: usize,
    request_timeout: Duration,
}

impl SandboxControlConnectionLimits {
    pub fn new(
        request_bytes: usize,
        response_bytes: usize,
        request_timeout: Duration,
    ) -> Result<Self, SandboxControlUdsError> {
        if request_bytes == 0 || response_bytes == 0 || request_timeout.is_zero() {
            return Err(SandboxControlUdsError::new(
                SandboxControlUdsStage::Configure,
                "sandbox control connection limits and timeout must be positive",
            ));
        }
        Instant::now().checked_add(request_timeout).ok_or_else(|| {
            SandboxControlUdsError::new(
                SandboxControlUdsStage::Configure,
                "sandbox control request timeout exceeds the platform clock range",
            )
        })?;
        Ok(Self {
            request_bytes,
            response_bytes,
            request_timeout,
        })
    }

    pub const fn request_bytes(self) -> usize {
        self.request_bytes
    }

    pub const fn response_bytes(self) -> usize {
        self.response_bytes
    }

    pub const fn request_timeout(self) -> Duration {
        self.request_timeout
    }
}

enum ConnectionPhase {
    Reading,
    AwaitingResponse,
    Writing,
}

pub(crate) struct SandboxControlConnection {
    id: u64,
    stream: UnixStream,
    request: Vec<u8>,
    expected_request_bytes: Option<usize>,
    response: Vec<u8>,
    response_written: usize,
    deadline: Instant,
    phase: ConnectionPhase,
}

impl SandboxControlConnection {
    pub(crate) fn new(
        id: u64,
        stream: UnixStream,
        limits: SandboxControlConnectionLimits,
    ) -> Result<Self, SandboxControlUdsError> {
        stream
            .set_nonblocking(true)
            .map_err(|error| io_error(SandboxControlUdsStage::Configure, error))?;
        let deadline = Instant::now()
            .checked_add(limits.request_timeout)
            .ok_or_else(|| {
                SandboxControlUdsError::new(
                    SandboxControlUdsStage::Configure,
                    "sandbox control request deadline overflow",
                )
            })?;
        Ok(Self {
            id,
            stream,
            request: Vec::with_capacity(limits.request_bytes.min(1024)),
            expected_request_bytes: None,
            response: Vec::new(),
            response_written: 0,
            deadline,
            phase: ConnectionPhase::Reading,
        })
    }

    pub(crate) const fn id(&self) -> u64 {
        self.id
    }

    pub(crate) fn raw_fd(&self) -> RawFd {
        self.stream.as_raw_fd()
    }

    pub(crate) fn poll_events(&self) -> i16 {
        match self.phase {
            ConnectionPhase::Reading => libc::POLLIN,
            ConnectionPhase::AwaitingResponse => 0,
            ConnectionPhase::Writing => libc::POLLOUT,
        }
    }

    pub(crate) fn expired(&self, now: Instant) -> bool {
        now >= self.deadline
    }

    pub(crate) fn deadline(&self) -> Instant {
        self.deadline
    }

    pub(crate) fn read_command(
        &mut self,
        codec: &SandboxControlCodec,
        limits: SandboxControlConnectionLimits,
    ) -> Result<Option<SandboxControlCommand>, SandboxControlUdsError> {
        if !matches!(self.phase, ConnectionPhase::Reading) {
            return Ok(None);
        }
        self.read_request(codec, limits)?;
        if !self.request_complete() {
            return Ok(None);
        }
        let command = codec.decode_command(&self.request)?;
        self.phase = ConnectionPhase::AwaitingResponse;
        Ok(Some(command))
    }

    pub(crate) fn set_response(
        &mut self,
        codec: &SandboxControlCodec,
        limits: SandboxControlConnectionLimits,
        response: &SandboxControlResponse,
    ) -> Result<(), SandboxControlUdsError> {
        if !matches!(self.phase, ConnectionPhase::AwaitingResponse) {
            return Ok(());
        }
        let encoded = codec.encode_response(response)?;
        if encoded.len() > limits.response_bytes {
            return Err(SandboxControlUdsError::new(
                SandboxControlUdsStage::Encode,
                "sandbox control response exceeds connection limit",
            ));
        }
        self.response = encoded;
        self.phase = ConnectionPhase::Writing;
        Ok(())
    }

    pub(crate) fn write_response(&mut self) -> Result<bool, SandboxControlUdsError> {
        if !matches!(self.phase, ConnectionPhase::Writing) {
            return Ok(false);
        }
        while self.response_written < self.response.len() {
            match self.stream.write(&self.response[self.response_written..]) {
                Ok(0) => return Ok(true),
                Ok(count) => self.response_written += count,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(false),
                Err(error) => return Err(io_error(SandboxControlUdsStage::Write, error)),
            }
        }
        Ok(true)
    }

    fn read_request(
        &mut self,
        codec: &SandboxControlCodec,
        limits: SandboxControlConnectionLimits,
    ) -> Result<(), SandboxControlUdsError> {
        loop {
            if self.request.len() == limits.request_bytes {
                return Err(SandboxControlUdsError::new(
                    SandboxControlUdsStage::Read,
                    "sandbox control request exceeds connection limit",
                ));
            }
            let remaining = limits.request_bytes - self.request.len();
            let mut buffer = [0_u8; 512];
            let read_bytes = remaining.min(buffer.len());
            match self.stream.read(&mut buffer[..read_bytes]) {
                Ok(0) if self.request_complete() => return Ok(()),
                Ok(0) => {
                    return Err(SandboxControlUdsError::new(
                        SandboxControlUdsStage::Read,
                        "sandbox control client closed a partial request",
                    ));
                }
                Ok(count) => {
                    self.request.extend_from_slice(&buffer[..count]);
                    if self.expected_request_bytes.is_none() {
                        self.expected_request_bytes = codec.frame_len(&self.request)?;
                        if self
                            .expected_request_bytes
                            .is_some_and(|size| size > limits.request_bytes)
                        {
                            return Err(SandboxControlUdsError::new(
                                SandboxControlUdsStage::Read,
                                "sandbox control request exceeds connection limit",
                            ));
                        }
                    }
                    if self.request_complete() {
                        return Ok(());
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(()),
                Err(error) => return Err(io_error(SandboxControlUdsStage::Read, error)),
            }
        }
    }

    fn request_complete(&self) -> bool {
        self.expected_request_bytes == Some(self.request.len())
    }
}

fn io_error(stage: SandboxControlUdsStage, error: std::io::Error) -> SandboxControlUdsError {
    SandboxControlUdsError::new(stage, error.to_string())
}
