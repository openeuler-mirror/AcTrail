use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::mpsc::{SyncSender, TrySendError, sync_channel};
use std::time::{Duration, Instant};

use sandbox_control::{
    MAX_SANDBOX_CONTROL_REJECTION_REASON_BYTES, SandboxControlCommand, SandboxControlPort,
    SandboxControlRejection, SandboxControlRejectionCode, SandboxControlResponse,
    SandboxControlStatus,
};

use crate::session::SessionWake;
use crate::session::SharedSessionStatus;

pub(crate) struct ControlRequest {
    pub(crate) command: SandboxControlCommand,
    pub(crate) response: SyncSender<SandboxControlResponse>,
    deadline: Instant,
    completion: Arc<AtomicU8>,
}

const PENDING: u8 = 0;
const CANCELLED: u8 = 1;
const COMMITTED: u8 = 2;

impl ControlRequest {
    pub(crate) fn permit_commit(&self) -> bool {
        if Instant::now() >= self.deadline {
            let _ = self.cancel();
            return false;
        }
        self.completion
            .compare_exchange(PENDING, COMMITTED, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    pub(crate) fn cancelled_or_expired(&self) -> bool {
        self.completion.load(Ordering::Acquire) == CANCELLED || Instant::now() >= self.deadline
    }

    fn cancel(&self) -> bool {
        self.completion
            .compare_exchange(PENDING, CANCELLED, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }
}

pub(crate) enum SessionCommand {
    Execute(ControlRequest),
    Shutdown,
}

#[derive(Clone)]
pub struct SandboxAgentControlHandle {
    pub(super) sender: SyncSender<SessionCommand>,
    status: Arc<SharedSessionStatus>,
    wake: Arc<SessionWake>,
    request_timeout: Duration,
}

impl SandboxAgentControlHandle {
    pub(super) fn new(
        sender: SyncSender<SessionCommand>,
        status: Arc<SharedSessionStatus>,
        wake: Arc<SessionWake>,
        request_timeout: Duration,
    ) -> Self {
        Self {
            sender,
            status,
            wake,
            request_timeout,
        }
    }

    pub(super) fn shutdown(&self) {
        self.wake.begin_command();
        match self.sender.try_send(SessionCommand::Shutdown) {
            Ok(()) => self.wake.notify(),
            Err(_) => self.wake.cancel_command(),
        }
    }
}

impl SandboxControlPort for SandboxAgentControlHandle {
    fn execute(&mut self, command: SandboxControlCommand) -> SandboxControlResponse {
        let (response, receiver) = sync_channel(1);
        let deadline = Instant::now()
            .checked_add(self.request_timeout)
            .expect("validated sandbox control timeout");
        let completion = Arc::new(AtomicU8::new(PENDING));
        let request = SessionCommand::Execute(ControlRequest {
            command,
            response,
            deadline,
            completion: Arc::clone(&completion),
        });
        self.wake.begin_command();
        match self.sender.try_send(request) {
            Ok(()) => {
                self.wake.notify();
                match receiver.recv_timeout(self.request_timeout) {
                    Ok(response) => response,
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => shutting_down(),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        if completion
                            .compare_exchange(
                                PENDING,
                                CANCELLED,
                                Ordering::AcqRel,
                                Ordering::Acquire,
                            )
                            .is_ok()
                        {
                            timed_out()
                        } else {
                            receiver.recv().unwrap_or_else(|_| shutting_down())
                        }
                    }
                }
            }
            Err(TrySendError::Full(_)) => {
                self.wake.cancel_command();
                rejected(
                    SandboxControlRejectionCode::Busy,
                    "sandbox connection control is busy",
                )
            }
            Err(TrySendError::Disconnected(_)) => {
                self.wake.cancel_command();
                shutting_down()
            }
        }
    }

    fn status(&self) -> SandboxControlStatus {
        self.status.snapshot()
    }
}

fn timed_out() -> SandboxControlResponse {
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

pub(crate) fn rejected(
    code: SandboxControlRejectionCode,
    message: impl Into<String>,
) -> SandboxControlResponse {
    let mut message = message.into();
    if message.len() > MAX_SANDBOX_CONTROL_REJECTION_REASON_BYTES {
        let mut end = MAX_SANDBOX_CONTROL_REJECTION_REASON_BYTES;
        while !message.is_char_boundary(end) {
            end -= 1;
        }
        message.truncate(end);
    }
    SandboxControlResponse::Rejected(
        SandboxControlRejection::new(code, message)
            .expect("sandbox runtime rejection messages are non-empty and bounded"),
    )
}
