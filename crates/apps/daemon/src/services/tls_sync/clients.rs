//! TLS-sync client acceptance, event reads and peer rejection.

use std::collections::BTreeMap;
use std::io::{ErrorKind, Read};
use std::os::fd::AsRawFd;
use std::time::SystemTime;

use control_contract::reply::ControlError;
use model_core::ids::TraceId;
use model_core::payload::PayloadSourceBoundary;
use model_core::trace::TlsEventToken;
use payload_event::RawPayloadStreamClose;
use storage_core::TlsFlowDiagnostic;
use tls_payload_core::PayloadDirection as SyncDirection;
use tls_payload_sync::{PlanLookupRequest, SyncEvent, SyncMessage};
use trace_runtime::registry::TraceRuntime;
use uds_control_server::PeerCredentials;

use super::frame_buffer::FrameBuffer;
use super::root_path::PeerRootHandle;
use super::{
    TlsSyncClient, TlsSyncDiagnostic, TlsSyncDrain, TlsSyncService, TraceConnections,
    payload_segment, stream_key, summary_segment, sync_event_error, tls_flow_reason_code,
};
use crate::peer_identity::{PeerIdentity, peer_error};

#[derive(Clone, Copy)]
pub(super) struct ReceiveLimits {
    pub(super) max_frame_bytes: usize,
    pub(super) read_max_bytes: usize,
    pub(super) buffer_retain_max_bytes: usize,
}

impl TlsSyncService {
    pub(in crate::services) fn drain(
        &mut self,
        trace_runtime: &TraceRuntime,
    ) -> Result<TlsSyncDrain, ControlError> {
        self.accept_ready_clients()?;
        let mut drain = TlsSyncDrain::default();
        for id in self.ready_clients()? {
            let mut client = self
                .clients
                .remove(&id)
                .expect("ready client is registered");
            let result = client.read_events(
                &mut self.read_buffer,
                self.receive_limits,
                &mut ConnectionAuthorization {
                    runtime: trace_runtime,
                    connections: &mut self.trace_connections,
                    maximum: self.trace_connection_max,
                },
                &mut drain,
            );
            let lookup = match result {
                Ok(ClientRead::Open) => {
                    self.clients.insert(id, client);
                    continue;
                }
                Ok(ClientRead::Closed) => None,
                Ok(ClientRead::PlanLookup(request)) => Some(request),
                Err(error) => {
                    audit_tls_peer_rejection(client.credentials, &error);
                    drain.diagnostics.push(TlsSyncDiagnostic {
                        trace_id: client.trace,
                        code: error.code,
                        message: error.message,
                    });
                    None
                }
            };
            self.unregister_client(&client)?;
            self.accept_paused = false;
            if let Some(trace) = client.trace {
                let observed_at = SystemTime::now();
                drain.payload_stream_closes.extend(
                    std::mem::take(&mut client.streams)
                        .into_iter()
                        .map(|(stream_key, process)| RawPayloadStreamClose {
                            trace_id: trace,
                            observed_at,
                            process,
                            source_boundary: PayloadSourceBoundary::TlsUserSpace,
                            stream_key,
                        }),
                );
                let connections = self
                    .trace_connections
                    .get_mut(&trace)
                    .expect("bound trace has connections");
                connections.active -= 1;
                if connections.active == 0 {
                    self.trace_connections.remove(&trace);
                }
            }
            if let Some(request) = lookup {
                let peer_root = PeerRootHandle::open(client.credentials.pid);
                let response = client.stream;
                if let Err(error) = self
                    .resolver
                    .as_ref()
                    .expect("listener has a resolver")
                    .submit_lookup(&request.binary, peer_root, response)
                {
                    drain.diagnostics.push(TlsSyncDiagnostic {
                        trace_id: client.trace,
                        code: error.code,
                        message: error.message,
                    });
                }
            }
        }
        Ok(drain)
    }

    fn ready_clients(&mut self) -> Result<Vec<u64>, ControlError> {
        let Some(epoll) = &self.client_epoll else {
            return Ok(Vec::new());
        };
        let count = unsafe {
            libc::epoll_wait(
                epoll.as_raw_fd(),
                self.ready_events.as_mut_ptr(),
                self.ready_events.len() as i32,
                0,
            )
        };
        if count < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == ErrorKind::Interrupted {
                return Ok(Vec::new());
            }
            return Err(ControlError::new("tls_sync_epoll", error.to_string()));
        }
        Ok(self.ready_events[..count as usize]
            .iter()
            .map(|event| event.u64)
            .collect())
    }

    fn unregister_client(&self, client: &TlsSyncClient) -> Result<(), ControlError> {
        let epoll = self.client_epoll.as_ref().expect("clients require epoll");
        let result = unsafe {
            libc::epoll_ctl(
                epoll.as_raw_fd(),
                libc::EPOLL_CTL_DEL,
                client.stream.as_raw_fd(),
                std::ptr::null_mut(),
            )
        };
        if result < 0 {
            return Err(ControlError::new(
                "tls_sync_epoll",
                std::io::Error::last_os_error().to_string(),
            ));
        }
        Ok(())
    }

    fn accept_ready_clients(&mut self) -> Result<(), ControlError> {
        let Some(listener) = &self.listener else {
            return Ok(());
        };
        loop {
            match listener.accept() {
                Ok((stream, _)) => {
                    self.accept_paused = false;
                    if self.clients.len() >= self.client_max {
                        drop(stream);
                        continue;
                    }
                    let credentials = match PeerCredentials::from_stream(&stream) {
                        Ok(credentials) => credentials,
                        Err(error) => {
                            tracing::warn!(
                                target: "actrail::peer_auth",
                                error = %error,
                                "rejected TLS-sync connection without peer credentials"
                            );
                            continue;
                        }
                    };
                    stream
                        .set_nonblocking(true)
                        .map_err(|error| ControlError::new("tls_sync_client", error.to_string()))?;
                    self.next_client += 1;
                    let mut event = libc::epoll_event {
                        events: libc::EPOLLIN as u32,
                        u64: self.next_client,
                    };
                    let epoll = self.client_epoll.as_ref().expect("listener requires epoll");
                    if unsafe {
                        libc::epoll_ctl(
                            epoll.as_raw_fd(),
                            libc::EPOLL_CTL_ADD,
                            stream.as_raw_fd(),
                            &mut event,
                        )
                    } < 0
                    {
                        return Err(ControlError::new(
                            "tls_sync_epoll",
                            std::io::Error::last_os_error().to_string(),
                        ));
                    }
                    self.clients.insert(
                        self.next_client,
                        TlsSyncClient {
                            trace: None,
                            stream,
                            credentials,
                            registration: None,
                            buffer: FrameBuffer::default(),
                            process_cache: BTreeMap::new(),
                            streams: BTreeMap::new(),
                        },
                    );
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    self.accept_paused = false;
                    return Ok(());
                }
                Err(error) if matches!(error.raw_os_error(), Some(libc::EMFILE | libc::ENFILE)) => {
                    if !self.accept_paused {
                        tracing::warn!(error = %error, "pausing TLS-sync accept while fd limit is reached");
                    }
                    self.accept_paused = true;
                    return Ok(());
                }
                Err(error) => {
                    return Err(ControlError::new("tls_sync_accept", error.to_string()));
                }
            }
        }
    }
}

enum ClientRead {
    Open,
    Closed,
    PlanLookup(PlanLookupRequest),
}

impl TlsSyncClient {
    fn read_events(
        &mut self,
        scratch: &mut [u8],
        limits: ReceiveLimits,
        authorization: &mut ConnectionAuthorization<'_>,
        drain: &mut TlsSyncDrain,
    ) -> Result<ClientRead, ControlError> {
        let mut read_bytes = 0;
        while read_bytes < limits.read_max_bytes {
            let read_limit = scratch.len().min(limits.read_max_bytes - read_bytes);
            match self.stream.read(&mut scratch[..read_limit]) {
                Ok(0) if self.buffer.is_empty() => {
                    return Ok(ClientRead::Closed);
                }
                Ok(0) => {
                    return Err(ControlError::new(
                        "tls_sync_event",
                        "truncated sync frame at EOF",
                    ));
                }
                Ok(read) => {
                    read_bytes += read;
                    self.buffer.append(&scratch[..read]);
                    match self.drain_complete_frames(limits, authorization, drain)? {
                        ClientRead::Open => {}
                        outcome => return Ok(outcome),
                    }
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => return Ok(ClientRead::Open),
                Err(error) => return Err(ControlError::new("tls_sync_read", error.to_string())),
            }
        }
        Ok(ClientRead::Open)
    }

    fn drain_complete_frames(
        &mut self,
        limits: ReceiveLimits,
        authorization: &mut ConnectionAuthorization<'_>,
        drain: &mut TlsSyncDrain,
    ) -> Result<ClientRead, ControlError> {
        while let Some(message) = self
            .buffer
            .next(limits.max_frame_bytes)
            .map_err(sync_event_error)?
        {
            let event = match message {
                SyncMessage::Register(registration) => {
                    if self.registration.replace(registration).is_some() {
                        return Err(peer_error(
                            "TLS-sync event transport registered more than once",
                        ));
                    }
                    continue;
                }
                SyncMessage::Event(event) => event,
                SyncMessage::PlanResponse(_) => {
                    return Err(ControlError::new(
                        "tls_sync_event",
                        "unexpected plan response from runtime",
                    ));
                }
                SyncMessage::PlanLookup(request) => {
                    // Plan lookup needs a live process root; buffered event reports do not.
                    PeerIdentity::resolve(self.credentials)?;
                    return Ok(ClientRead::PlanLookup(request));
                }
            };
            let trace_id = match &event {
                SyncEvent::Payload(event) => TraceId::new(event.trace_id),
                SyncEvent::Summary(event) => TraceId::new(event.trace_id),
                SyncEvent::Decision(event) => TraceId::new(event.trace_id),
            };
            if !authorization.bind(self, trace_id, &mut drain.diagnostics)? {
                return Ok(ClientRead::Closed);
            }
            let segment = match event {
                SyncEvent::Payload(event) => payload_segment(event, &mut self.process_cache)?,
                SyncEvent::Summary(event) => {
                    drain.flow_diagnostics.push(TlsFlowDiagnostic {
                        trace_id: TraceId::new(event.trace_id),
                        stream_key: stream_key(event.pid, event.stream_key),
                        direction: match event.direction {
                            SyncDirection::Outbound => 0,
                            SyncDirection::Inbound => 1,
                        },
                        reason_code: tls_flow_reason_code(&event.reason),
                        observed_size: event.observed_size,
                        emitted_size: event.emitted_size,
                        emitted_at: SystemTime::now(),
                    });
                    summary_segment(event, &mut self.process_cache)?
                }
                SyncEvent::Decision(_) => None,
            };
            if let Some(segment) = segment {
                self.streams
                    .entry(segment.stream_key.clone())
                    .or_insert_with(|| segment.process.clone());
                drain.payload_segments.push(segment);
            }
        }
        self.buffer.compact(limits.buffer_retain_max_bytes);
        Ok(ClientRead::Open)
    }
}

struct ConnectionAuthorization<'a> {
    runtime: &'a TraceRuntime,
    connections: &'a mut BTreeMap<TraceId, TraceConnections>,
    maximum: u32,
}

impl ConnectionAuthorization<'_> {
    fn bind(
        &mut self,
        client: &mut TlsSyncClient,
        trace_id: TraceId,
        diagnostics: &mut Vec<TlsSyncDiagnostic>,
    ) -> Result<bool, ControlError> {
        if let Some(bound) = client.trace {
            if bound != trace_id {
                return Err(peer_error(format!(
                    "TLS-sync connection for trace {bound} cannot report trace {trace_id}"
                )));
            }
            return Ok(true);
        }
        authorize_sync_event(client, self.runtime, trace_id)?;
        let connections = self.connections.entry(trace_id).or_default();
        if connections.active >= self.maximum {
            connections.rejected += 1;
            if connections.rejected == 1 {
                diagnostics.push(TlsSyncDiagnostic {
                    trace_id: Some(trace_id),
                    code: "tls_sync_trace_connection_limit".to_string(),
                    message: format!("TLS-sync trace connection limit reached: {}/{}; closing excess process transports", connections.active, self.maximum),
                });
            }
            return Ok(false);
        }
        connections.active += 1;
        client.trace = Some(trace_id);
        Ok(true)
    }
}

fn authorize_sync_event(
    client: &TlsSyncClient,
    trace_runtime: &TraceRuntime,
    trace_id: TraceId,
) -> Result<(), ControlError> {
    let trace = trace_runtime.get_trace(trace_id).ok_or_else(|| {
        peer_error(format!(
            "TLS-sync event references unknown trace {trace_id}"
        ))
    })?;
    let registration = client
        .registration
        .as_ref()
        .ok_or_else(|| peer_error("TLS-sync events require a connection registration"))?;
    let token = TlsEventToken::from_hex(&registration.token)
        .ok_or_else(|| peer_error("invalid TLS-sync registration token"))?;
    if token != trace.tls_event_token {
        return Err(peer_error(format!(
            "TLS-sync connection is not authorized for {trace_id}"
        )));
    }
    Ok(())
}

fn audit_tls_peer_rejection(credentials: PeerCredentials, error: &ControlError) {
    tracing::warn!(
        target: "actrail::peer_auth",
        peer_pid = credentials.pid,
        peer_uid = credentials.uid,
        peer_gid = credentials.gid,
        error_code = %error.code,
        error = %error.message,
        "closed rejected TLS-sync peer"
    );
}
