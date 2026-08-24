use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, sync_channel};
use std::thread::Thread;
use std::time::Instant;

use sandbox_control::{
    SandboxConnectResponse, SandboxControlCommand, SandboxControlRejectionCode,
    SandboxControlResponse, SandboxEndpoint,
};
use sandbox_observation::Observation;

use super::protocol::SessionProtocol;
use super::status::SharedSessionStatus;
use super::wake::SessionWake;
use crate::daemon::{BaselineRequest, ControlRequest, SessionCommand, rejected};
use crate::delivery::{ConnectionGate, ConnectionGeneration, DeliveryEnvelope, DeliveryQueue};
use crate::status::DaemonMetrics;
use crate::{SandboxAgentConfig, SandboxConnection, SandboxTransportFactory};

pub(crate) struct SessionOwner {
    config: SandboxAgentConfig,
    transport: Arc<dyn SandboxTransportFactory>,
    gate: Arc<ConnectionGate>,
    delivery: DeliveryQueue,
    commands: Receiver<SessionCommand>,
    baseline: SyncSender<BaselineRequest>,
    io_thread: Thread,
    status: Arc<SharedSessionStatus>,
    metrics: Arc<DaemonMetrics>,
    stop: Arc<AtomicBool>,
    wake: Arc<SessionWake>,
    protocol: SessionProtocol,
    active: Option<ActiveSession>,
    reconnect: Option<ReconnectTarget>,
    next_generation: u64,
    pending: Vec<Observation>,
}

struct ActiveSession {
    endpoint: SandboxEndpoint,
    generation: ConnectionGeneration,
    sb_id: u32,
    connection: Box<dyn SandboxConnection>,
    next_sequence: u64,
    last_write: Instant,
}

#[derive(Clone, Copy)]
struct ReconnectTarget {
    endpoint: SandboxEndpoint,
    next_attempt: Instant,
}

enum EstablishError {
    Connect(io::Error),
    Handshake(io::Error),
    Baseline(io::Error),
    Superseded,
}

impl SessionOwner {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        config: SandboxAgentConfig,
        transport: Arc<dyn SandboxTransportFactory>,
        gate: Arc<ConnectionGate>,
        delivery: DeliveryQueue,
        commands: Receiver<SessionCommand>,
        baseline: SyncSender<BaselineRequest>,
        io_thread: Thread,
        status: Arc<SharedSessionStatus>,
        metrics: Arc<DaemonMetrics>,
        stop: Arc<AtomicBool>,
        wake: Arc<SessionWake>,
        pending: Vec<Observation>,
    ) -> Self {
        Self {
            config,
            transport,
            gate,
            delivery,
            commands,
            baseline,
            io_thread,
            status,
            metrics,
            stop,
            wake,
            protocol: SessionProtocol::new(),
            active: None,
            reconnect: None,
            next_generation: 0,
            pending,
        }
    }

    pub(crate) fn run(mut self) {
        self.wake.bind_current();
        while !self.stop.load(Ordering::Acquire) {
            match self.commands.try_recv() {
                Ok(command) => {
                    self.wake.command_received();
                    if !self.handle_command(command) {
                        break;
                    }
                    continue;
                }
                Err(TryRecvError::Disconnected) => break,
                Err(TryRecvError::Empty) => {}
            }

            if self.active.is_some() {
                self.run_connected_cycle();
            } else if self.reconnect.is_some() {
                self.run_reconnect_cycle();
            } else {
                match self.commands.recv() {
                    Ok(command) => {
                        self.wake.command_received();
                        if !self.handle_command(command) {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        }
        self.disconnect_all();
        self.status.stopping();
    }

    fn handle_command(&mut self, command: SessionCommand) -> bool {
        match command {
            SessionCommand::Execute(request) => {
                let response = self.execute(&request);
                let _ = request.response.send(response);
                true
            }
            SessionCommand::Shutdown => false,
        }
    }

    fn execute(&mut self, request: &ControlRequest) -> SandboxControlResponse {
        if self.stop.load(Ordering::Acquire) {
            return shutting_down();
        }
        if request.cancelled_or_expired() {
            return control_timeout();
        }
        match request.command {
            SandboxControlCommand::Connect(command) => self.connect(request, command.endpoint()),
        }
    }

    fn connect(
        &mut self,
        request: &ControlRequest,
        endpoint: SandboxEndpoint,
    ) -> SandboxControlResponse {
        if let Some(active) = &self.active
            && active.endpoint == endpoint
        {
            if !request.permit_commit() {
                return control_timeout();
            }
            return SandboxControlResponse::Connect(SandboxConnectResponse::new(
                endpoint,
                active.sb_id,
                active.generation.get(),
                true,
            ));
        }

        self.disconnect_all();
        self.status.connecting(endpoint);
        match self.establish(endpoint) {
            Ok(active) => {
                if self.stop.load(Ordering::Acquire) {
                    self.status.disconnected();
                    return shutting_down();
                }
                if !request.permit_commit() {
                    self.status.disconnected();
                    return control_timeout();
                }
                let response = SandboxConnectResponse::new(
                    endpoint,
                    active.sb_id,
                    active.generation.get(),
                    false,
                );
                if self.publish_active(active) {
                    SandboxControlResponse::Connect(response)
                } else {
                    shutting_down()
                }
            }
            Err(error) => {
                self.status.disconnected();
                self.rejection(error)
            }
        }
    }

    fn run_connected_cycle(&mut self) {
        let mut active = self
            .active
            .take()
            .expect("connected cycle requires session");
        if self.stop.load(Ordering::Acquire) {
            self.pending.clear();
            self.delivery.discard_all();
            return;
        }
        while self.pending.len() < self.config.batch_max_observations {
            match self.delivery.try_recv() {
                Ok(envelope) => self.admit(active.generation, envelope),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.stop.store(true, Ordering::Release);
                    return;
                }
            }
        }

        if self.stop.load(Ordering::Acquire) {
            self.pending.clear();
            self.delivery.discard_all();
            return;
        }

        let send_result = if !self.pending.is_empty() {
            self.protocol.send_batch(
                &mut *active.connection,
                active.next_sequence,
                &mut self.pending,
            )
        } else if active.last_write.elapsed() >= self.config.max_silence_interval {
            self.protocol.send_heartbeat(&mut *active.connection)
        } else {
            let wait = self
                .config
                .max_silence_interval
                .saturating_sub(active.last_write.elapsed());
            self.active = Some(active);
            self.wake.wait(wait);
            return;
        };

        if send_result.is_err() {
            self.pending.clear();
            self.begin_reconnect(active.endpoint);
            return;
        }
        if !self.pending.is_empty() {
            self.pending.clear();
            active.next_sequence = active.next_sequence.wrapping_add(1).max(1);
            self.metrics.record_sent_batch();
        }
        active.last_write = Instant::now();
        self.active = Some(active);
    }

    fn run_reconnect_cycle(&mut self) {
        let target = self.reconnect.take().expect("reconnect target required");
        let now = Instant::now();
        if now < target.next_attempt {
            self.reconnect = Some(target);
            self.wake
                .wait(target.next_attempt.saturating_duration_since(now));
            return;
        }

        match self.establish(target.endpoint) {
            Ok(active) => {
                if self.stop.load(Ordering::Acquire) {
                    self.status.disconnected();
                    return;
                }
                if self.publish_active(active) {
                    self.metrics.record_reconnect();
                }
            }
            Err(EstablishError::Superseded) => {
                self.status.reconnecting(target.endpoint);
                self.reconnect = Some(target);
            }
            Err(_) => {
                self.metrics.record_reconnect_failure();
                self.status.reconnecting(target.endpoint);
                self.reconnect = Some(ReconnectTarget {
                    endpoint: target.endpoint,
                    next_attempt: Instant::now() + self.config.reconnect_interval,
                });
            }
        }
    }

    fn establish(&mut self, endpoint: SandboxEndpoint) -> Result<ActiveSession, EstablishError> {
        let mut connection = self
            .transport
            .connect(endpoint)
            .map_err(EstablishError::Connect)?;
        self.ensure_current_attempt()?;
        let sb_id = self
            .protocol
            .handshake(&mut *connection)
            .map_err(EstablishError::Handshake)?;
        self.ensure_current_attempt()?;
        self.establish_io_baseline()
            .map_err(EstablishError::Baseline)?;
        self.ensure_current_attempt()?;
        self.delivery.discard_all();
        let generation = self.advance_generation();
        Ok(ActiveSession {
            endpoint,
            generation,
            sb_id,
            connection,
            next_sequence: 1,
            last_write: Instant::now(),
        })
    }

    fn ensure_current_attempt(&self) -> Result<(), EstablishError> {
        if self.stop.load(Ordering::Acquire) || self.wake.command_pending() {
            Err(EstablishError::Superseded)
        } else {
            Ok(())
        }
    }

    fn establish_io_baseline(&self) -> io::Result<()> {
        let (response, receiver) = sync_channel(1);
        self.baseline
            .send(BaselineRequest { response })
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "I/O worker stopped"))?;
        self.io_thread.unpark();
        receiver
            .recv()
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "I/O baseline reply lost"))?
    }

    fn publish_active(&mut self, active: ActiveSession) -> bool {
        if self.stop.load(Ordering::Acquire) {
            self.status.disconnected();
            return false;
        }
        self.metrics.set_sb_id(active.sb_id);
        self.gate.enable(active.generation);
        self.status
            .connected(active.endpoint, active.sb_id, active.generation.get());
        if self.stop.load(Ordering::Acquire) {
            self.gate.disable();
            self.metrics.set_sb_id(0);
            self.status.disconnected();
            return false;
        }
        self.active = Some(active);
        self.reconnect = None;
        true
    }

    fn begin_reconnect(&mut self, endpoint: SandboxEndpoint) {
        self.gate.disable();
        self.metrics.set_sb_id(0);
        self.delivery.discard_all();
        self.status.reconnecting(endpoint);
        self.active = None;
        self.reconnect = Some(ReconnectTarget {
            endpoint,
            next_attempt: Instant::now(),
        });
    }

    fn disconnect_all(&mut self) {
        self.gate.disable();
        self.metrics.set_sb_id(0);
        self.pending.clear();
        self.active = None;
        self.reconnect = None;
        self.delivery.discard_all();
    }

    fn admit(&mut self, generation: ConnectionGeneration, envelope: DeliveryEnvelope) {
        if envelope.generation == generation {
            self.pending.push(envelope.observation);
        }
    }

    fn advance_generation(&mut self) -> ConnectionGeneration {
        self.next_generation = self.next_generation.wrapping_add(1).max(1);
        ConnectionGeneration::new(self.next_generation).expect("generation is non-zero")
    }

    fn rejection(&self, error: EstablishError) -> SandboxControlResponse {
        let (code, message) = match error {
            EstablishError::Connect(error) => (
                SandboxControlRejectionCode::ConnectFailed,
                format!("cannot connect sandbox VSOCK endpoint: {error}"),
            ),
            EstablishError::Handshake(error) => (
                SandboxControlRejectionCode::HandshakeFailed,
                format!("sandbox VSOCK handshake failed: {error}"),
            ),
            EstablishError::Baseline(error) => (
                SandboxControlRejectionCode::ConnectFailed,
                format!("cannot establish sandbox I/O baseline: {error}"),
            ),
            EstablishError::Superseded => (
                SandboxControlRejectionCode::Busy,
                "sandbox connection attempt was superseded".to_string(),
            ),
        };
        rejected(code, message)
    }
}

fn control_timeout() -> SandboxControlResponse {
    rejected(
        SandboxControlRejectionCode::ConnectFailed,
        "sandbox connection control timed out",
    )
}

fn shutting_down() -> SandboxControlResponse {
    rejected(
        SandboxControlRejectionCode::ShuttingDown,
        "sandbox daemon is shutting down",
    )
}
