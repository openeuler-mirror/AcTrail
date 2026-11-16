//! Releases for the TLS-sync runtimes of checkpoint launches. Such a runtime holds each response
//! until the daemon has processed what the process wrote before it on the same connection. The
//! release stops short of an LLM request of a watched Trace until its checkpoint work is done.

use std::collections::BTreeMap;
use std::io::{ErrorKind, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;

use model_core::ids::TraceId;
use tls_payload_core::PayloadDirection as SyncDirection;
use tls_payload_sync::{FrameCodec, SyncEvent};

use super::TlsSyncService;

/// The releases of one runtime connection.
#[derive(Default)]
pub(super) struct ClientReleases {
    /// Pending outbound progress and checkpoint holds, by process id and TLS connection.
    streams: BTreeMap<(u32, u64), StreamRelease>,
    trace_id: Option<TraceId>,
    outbox: Vec<u8>,
    ended: bool,
}

#[derive(Default)]
struct StreamRelease {
    /// Highest outbound sequence processed.
    seen: u64,
    released: Option<u64>,
    /// Number of LLM requests at each sequence still waiting for checkpoint work.
    holds: BTreeMap<u64, usize>,
}

impl ClientReleases {
    /// Notes an outbound event the daemon processed. A runtime whose Trace is not watched, for
    /// example after a disconnect, stops waiting.
    pub(super) fn observe(
        &mut self,
        trace_id: TraceId,
        watched: bool,
        pid: u32,
        connection: u64,
        sequence: u64,
    ) {
        if !watched {
            return self.end();
        }
        self.trace_id = Some(trace_id);
        let stream = self.streams.entry((pid, connection)).or_default();
        stream.seen = stream.seen.max(sequence);
    }

    pub(super) fn carries(&self, trace_id: TraceId) -> bool {
        self.trace_id == Some(trace_id)
    }

    /// Keeps the release before `sequence`; false once it was released.
    pub(super) fn hold(&mut self, pid: u32, connection: u64, sequence: u64) -> bool {
        let Some(stream) = self.streams.get_mut(&(pid, connection)) else {
            return false;
        };
        if stream.released.is_some_and(|released| released >= sequence) {
            return false;
        }
        *stream.holds.entry(sequence).or_default() += 1;
        true
    }

    pub(super) fn unhold(&mut self, pid: u32, connection: u64, sequence: u64) {
        if let Some(stream) = self.streams.get_mut(&(pid, connection))
            && let Some(count) = stream.holds.get_mut(&sequence)
        {
            *count -= 1;
            if *count == 0 {
                stream.holds.remove(&sequence);
            }
        }
    }

    /// The runtime stops waiting, here and in every later request.
    pub(super) fn end(&mut self) {
        if self.ended {
            return;
        }
        self.ended = true;
        self.streams.clear();
        self.outbox.extend(FrameCodec::hold_end_frame());
    }

    /// Called after the batch's LLM requests have registered their holds. Queues releases, then
    /// retires streams without holds: their progress now belongs to the outbox, and later outbound
    /// events create a fresh entry. Only held streams need to remember an earlier release.
    pub(super) fn queue(&mut self) {
        let outbox = &mut self.outbox;
        self.streams.retain(|&(_, connection), stream| {
            let target = match stream.holds.first_key_value() {
                Some((&hold, _)) => hold.checked_sub(1),
                None => Some(stream.seen),
            };
            if let Some(sequence) = target
                && target > stream.released
            {
                outbox.extend(FrameCodec::release_frame(connection, sequence));
                stream.released = target;
            }
            !stream.holds.is_empty()
        });
    }

    /// Writes as much as the socket takes; false when the connection is gone. A runtime reads its
    /// releases as they come, so one that leaves too many unread has its connection closed: the
    /// daemon's next read sees the end and closes the client, and the runtime stops waiting.
    pub(super) fn flush(&mut self, mut stream: &UnixStream, max_bytes: usize) -> bool {
        while !self.outbox.is_empty() {
            match stream.write(&self.outbox) {
                Ok(0) => return false,
                Ok(written) => {
                    self.outbox.drain(..written);
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    if self.outbox.len() <= max_bytes {
                        return true;
                    }
                    self.outbox.clear();
                    let _ = stream.shutdown(Shutdown::Both);
                    return false;
                }
                Err(error) if error.kind() == ErrorKind::Interrupted => {}
                Err(_) => return false,
            }
        }
        true
    }
}

/// An LLM request whose response waits for checkpoint work.
pub(super) struct Hold {
    pub(super) client: u64,
    pid: u32,
    connection: u64,
    sequence: u64,
}

impl TlsSyncService {
    /// Keeps back the response to an LLM request whose last outbound event is `sequence`, until
    /// `release`. None when the request is not on a connection whose runtime waits.
    pub(in crate::services) fn hold_request(
        &mut self,
        trace_id: TraceId,
        stream_key: &str,
        sequence: u64,
    ) -> Option<u64> {
        let (pid, connection) = parse_stream_key(stream_key)?;
        let client = self
            .clients
            .values_mut()
            .filter(|client| client.trace == Some(trace_id))
            .find_map(|client| {
                client
                    .releases
                    .hold(pid, connection, sequence)
                    .then_some(client.id)
            })?;
        self.next_hold += 1;
        self.holds.insert(
            self.next_hold,
            Hold {
                client,
                pid,
                connection,
                sequence,
            },
        );
        Some(self.next_hold)
    }

    /// Lets the response of a held LLM request reach the Agent.
    pub(in crate::services) fn release(&mut self, hold: u64) {
        let Some(hold) = self.holds.remove(&hold) else {
            return;
        };
        if let Some(client) = self.clients.get_mut(&hold.client) {
            client
                .releases
                .unhold(hold.pid, hold.connection, hold.sequence);
            client.releases.queue();
            client
                .releases
                .flush(&client.stream, self.release_outbox_max_bytes);
        }
    }

    /// The runtimes of an ended Trace stop waiting.
    pub(in crate::services) fn end_trace(&mut self, trace_id: TraceId) {
        for client in self.clients.values_mut() {
            if client.releases.carries(trace_id) {
                client.releases.end();
                client
                    .releases
                    .flush(&client.stream, self.release_outbox_max_bytes);
            }
        }
        let clients = &self.clients;
        self.holds.retain(|_, hold| {
            clients
                .get(&hold.client)
                .is_some_and(|client| !client.releases.carries(trace_id))
        });
    }

    /// Sends the releases of everything processed since the last call.
    pub(in crate::services) fn flush_releases(&mut self) {
        for client in self.clients.values_mut() {
            client.releases.queue();
            client
                .releases
                .flush(&client.stream, self.release_outbox_max_bytes);
        }
    }
}

/// The Trace, process, connection and sequence of an event carrying bytes the process wrote.
pub(super) fn outbound_progress(event: &SyncEvent) -> Option<(TraceId, u32, u64, u64)> {
    let (trace_id, pid, direction, connection, sequence) = match event {
        SyncEvent::Payload(event) => (
            event.trace_id,
            event.pid,
            event.direction,
            event.stream_key,
            event.sequence,
        ),
        SyncEvent::Summary(event) => (
            event.trace_id,
            event.pid,
            event.direction,
            event.stream_key,
            event.sequence,
        ),
        SyncEvent::Decision(_) => return None,
    };
    (direction == SyncDirection::Outbound).then_some((
        TraceId::new(trace_id),
        pid,
        connection,
        sequence,
    ))
}

fn parse_stream_key(stream_key: &str) -> Option<(u32, u64)> {
    let (pid, connection) = stream_key.strip_prefix("tls-sync:")?.split_once(':')?;
    Some((pid.parse().ok()?, u64::from_str_radix(connection, 16).ok()?))
}
