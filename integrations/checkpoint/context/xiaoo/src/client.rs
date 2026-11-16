//! Exchanges one bounded JSON request per connection with the Xiaoo socket.

use std::{
    io::{BufRead, BufReader, Write},
    os::fd::OwnedFd,
    os::unix::net::UnixStream,
    time::{Duration, Instant},
};

use serde::de::DeserializeOwned;
use socket2::{Domain, SockAddr, Socket, Type};

use crate::{
    config::Config,
    error::Error,
    protocol::{Request, Response, RestoreMode},
};

pub(super) struct Client {
    config: Config,
}

impl Client {
    pub fn new(config: Config) -> Self {
        Self { config }
    }

    pub fn restore_mode(&self) -> RestoreMode {
        self.config.restore_mode
    }

    pub fn request<T: DeserializeOwned>(&self, request: &Request<'_>) -> Result<T, Error> {
        let started = Instant::now();
        let address = SockAddr::unix(&self.config.endpoint)?;
        let socket = Socket::new(Domain::UNIX, Type::STREAM, None)?;
        socket.connect_timeout(&address, self.remaining(started)?)?;
        let socket: OwnedFd = socket.into();
        let mut stream = UnixStream::from(socket);
        let mut bytes =
            serde_json::to_vec(request).map_err(|error| Error::Operation(error.to_string()))?;
        bytes.push(b'\n');
        // One deadline covers connect, writes, and response reads.
        let mut pending = bytes.as_slice();
        while !pending.is_empty() {
            stream.set_write_timeout(Some(self.remaining(started)?))?;
            let written = match stream.write(pending) {
                Ok(written) => written,
                // Retry interrupted I/O within the original request deadline.
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error.into()),
            };
            if written == 0 {
                return Err(std::io::Error::from(std::io::ErrorKind::WriteZero).into());
            }
            pending = &pending[written..];
        }
        let mut reader = BufReader::new(stream);
        let mut response = Vec::new();
        // Bound allocation while reading through the first newline.
        loop {
            reader
                .get_ref()
                .set_read_timeout(Some(self.remaining(started)?))?;
            let chunk = match reader.fill_buf() {
                Ok(chunk) => chunk,
                // Keep accumulated bytes and recheck the deadline after interruption.
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error.into()),
            };
            if chunk.is_empty() {
                return Err(Error::Operation(
                    "Xiaoo closed the connection before a complete response".into(),
                ));
            }
            let newline = chunk.iter().position(|byte| *byte == b'\n');
            let length = newline.map_or(chunk.len(), |index| index + 1);
            if length
                > self
                    .config
                    .max_response_bytes
                    .saturating_sub(response.len())
            {
                return Err(Error::Operation(
                    "Xiaoo response exceeds max_response_bytes".into(),
                ));
            }
            response.extend_from_slice(&chunk[..length]);
            reader.consume(length);
            if newline.is_some() {
                break;
            }
        }
        let response: Response = serde_json::from_slice(&response)
            .map_err(|error| Error::Operation(format!("invalid Xiaoo response: {error}")))?;
        if !response.ok {
            return Err(Error::Operation(response.error.unwrap_or_else(|| {
                "Xiaoo rejected the request without an error message".into()
            })));
        }
        serde_json::from_value(response.result)
            .map_err(|error| Error::Operation(format!("invalid Xiaoo result: {error}")))
    }

    fn remaining(&self, started: Instant) -> Result<Duration, Error> {
        self.config
            .request_timeout
            .checked_sub(started.elapsed())
            .filter(|duration| !duration.is_zero())
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "Xiaoo checkpoint request timed out",
                )
                .into()
            })
    }
}
