//! Runtime-side sync event writer.

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::Path;

use crate::{SyncEvent, SyncResult, encode_event_line};

#[derive(Debug)]
pub struct EventClient {
    stream: UnixStream,
}

impl EventClient {
    pub fn connect(path: &Path) -> SyncResult<Self> {
        Ok(Self {
            stream: UnixStream::connect(path)?,
        })
    }

    pub fn send(&mut self, event: &SyncEvent) -> SyncResult<()> {
        self.stream.write_all(&encode_event_line(event))?;
        Ok(())
    }
}
