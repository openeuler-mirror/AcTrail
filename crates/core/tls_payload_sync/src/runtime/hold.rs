//! Response holding of checkpoint launches. A TLS read waits until the daemon has released the
//! outbound events the process queued on that connection before the read; the daemon keeps back
//! the release of an LLM request until its checkpoint work is done.

use std::collections::BTreeMap;
use std::io::{ErrorKind, Read};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};

use crate::{FrameCodec, SyncMessage};

#[derive(Debug, Default)]
pub struct ResponseHold {
    state: Mutex<HoldState>,
    released: Condvar,
}

#[derive(Debug, Default)]
struct HoldState {
    /// Sequence of the last outbound event queued, by TLS connection.
    sent: BTreeMap<u64, u64>,
    /// Sequence up to which the daemon released, by TLS connection.
    released: BTreeMap<u64, u64>,
    /// The daemon ended the hold or cannot be reached; nothing waits any more.
    ended: bool,
}

impl ResponseHold {
    /// Waits until the daemon released what this process queued on `stream_key` before the call.
    pub fn wait(&self, stream_key: u64) {
        let mut state = self.state();
        let Some(&target) = state.sent.get(&stream_key) else {
            return;
        };
        while !state.ended
            && state
                .released
                .get(&stream_key)
                .is_none_or(|released| *released < target)
        {
            state = self
                .released
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    pub(super) fn sent(&self, stream_key: u64, sequence: u64) {
        let mut state = self.state();
        let last = state.sent.entry(stream_key).or_insert(sequence);
        *last = (*last).max(sequence);
    }

    pub(super) fn end(&self) {
        self.state().ended = true;
        self.released.notify_all();
    }

    /// Applies the daemon's releases until it ends the hold or the connection closes.
    pub(super) fn read_releases(
        &self,
        mut stream: UnixStream,
        max_frame_bytes: usize,
        _registration: super::FdRegistration,
    ) {
        let mut buffer = Vec::new();
        let mut chunk = [0_u8; 4096];
        loop {
            match stream.read(&mut chunk) {
                Ok(0) => break,
                Ok(read) => buffer.extend_from_slice(&chunk[..read]),
                // The writer shares the non-blocking file description.
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    wait_readable(&stream);
                    continue;
                }
                Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
            loop {
                let header = match FrameCodec::parse_header(&buffer, max_frame_bytes) {
                    Ok(Some(header)) if buffer.len() >= header.frame_len() => header,
                    Ok(_) => break,
                    Err(_) => return self.end(),
                };
                let body = &buffer[FrameCodec::HEADER_LEN..header.frame_len()];
                match FrameCodec::decode(header, body) {
                    Ok(SyncMessage::Release {
                        stream_key,
                        sequence,
                    }) => self.release(stream_key, sequence),
                    // A hold end, or anything the daemon does not send.
                    _ => return self.end(),
                }
                buffer.drain(..header.frame_len());
            }
        }
        self.end();
    }

    fn release(&self, stream_key: u64, sequence: u64) {
        let mut state = self.state();
        let released = state.released.entry(stream_key).or_insert(sequence);
        *released = (*released).max(sequence);
        drop(state);
        self.released.notify_all();
    }

    fn state(&self) -> MutexGuard<'_, HoldState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn wait_readable(stream: &UnixStream) {
    let mut descriptor = libc::pollfd {
        fd: stream.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    unsafe { libc::poll(&mut descriptor, 1, -1) };
}
