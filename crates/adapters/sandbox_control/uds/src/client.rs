//! One-request/one-response client used by `actrail-sb connect`.

use std::io::{Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use sandbox_control::{SandboxControlCommand, SandboxControlResponse};

use crate::{SandboxControlCodec, SandboxControlUdsError, SandboxControlUdsStage};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SandboxControlUdsClientConfig {
    socket_path: PathBuf,
    request_timeout: Duration,
}

impl SandboxControlUdsClientConfig {
    pub fn new(
        socket_path: impl Into<PathBuf>,
        request_timeout: Duration,
    ) -> Result<Self, SandboxControlUdsError> {
        let socket_path = socket_path.into();
        if !socket_path.is_absolute() {
            return Err(SandboxControlUdsError::new(
                SandboxControlUdsStage::Configure,
                "sandbox control socket path must be absolute",
            ));
        }
        if request_timeout.is_zero() {
            return Err(SandboxControlUdsError::new(
                SandboxControlUdsStage::Configure,
                "sandbox control request timeout must be positive",
            ));
        }
        Ok(Self {
            socket_path,
            request_timeout,
        })
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    pub const fn request_timeout(&self) -> Duration {
        self.request_timeout
    }
}

pub struct SandboxControlUdsClient {
    config: SandboxControlUdsClientConfig,
    codec: SandboxControlCodec,
}

impl SandboxControlUdsClient {
    pub const fn new(config: SandboxControlUdsClientConfig, codec: SandboxControlCodec) -> Self {
        Self { config, codec }
    }

    pub fn send(
        &self,
        command: &SandboxControlCommand,
    ) -> Result<SandboxControlResponse, SandboxControlUdsError> {
        let request = self.codec.encode_command(command)?;
        let mut stream = UnixStream::connect(&self.config.socket_path)
            .map_err(|error| io_error(SandboxControlUdsStage::Connect, error))?;
        stream
            .set_read_timeout(Some(self.config.request_timeout))
            .and_then(|_| stream.set_write_timeout(Some(self.config.request_timeout)))
            .map_err(|error| io_error(SandboxControlUdsStage::Configure, error))?;
        stream
            .write_all(&request)
            .and_then(|_| stream.shutdown(Shutdown::Write))
            .map_err(|error| io_error(SandboxControlUdsStage::Write, error))?;
        let response = self.read_response(&mut stream)?;
        self.codec.decode_response(&response)
    }

    pub const fn config(&self) -> &SandboxControlUdsClientConfig {
        &self.config
    }

    fn read_response(&self, stream: &mut UnixStream) -> Result<Vec<u8>, SandboxControlUdsError> {
        let mut header = [0_u8; 8];
        stream
            .read_exact(&mut header)
            .map_err(|error| io_error(SandboxControlUdsStage::Read, error))?;
        let frame_len = self
            .codec
            .frame_len(&header)?
            .expect("complete fixed-size header");
        let mut response = Vec::with_capacity(frame_len);
        response.extend_from_slice(&header);
        response.resize(frame_len, 0);
        stream
            .read_exact(&mut response[header.len()..])
            .map_err(|error| io_error(SandboxControlUdsStage::Read, error))?;
        Ok(response)
    }
}

fn io_error(stage: SandboxControlUdsStage, error: std::io::Error) -> SandboxControlUdsError {
    SandboxControlUdsError::new(stage, error.to_string())
}
