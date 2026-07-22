use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::Arc;
use std::sync::mpsc::{RecvTimeoutError, SyncSender, TrySendError, sync_channel};
use std::time::{Duration, Instant};

use control_contract::reply::ControlError;
use model_core::ids::TraceId;
use plugin_system::{
    PluginRuntimeError, PostTraceHost, TraceAnalysisActionPage, TraceAnalysisContext,
    TraceFileState,
};

#[derive(Clone)]
pub(crate) struct PostTraceHostClient {
    scope: PluginScope,
    request_sender: SyncSender<BrokerRequest>,
    signal: Arc<EventSignal>,
    reply_timeout: Duration,
    file_state_timeout: Duration,
}

impl PostTraceHostClient {
    pub(super) fn new(
        scope: PluginScope,
        request_sender: SyncSender<BrokerRequest>,
        signal: Arc<EventSignal>,
        reply_timeout: Duration,
        file_state_timeout: Duration,
    ) -> Self {
        Self {
            scope,
            request_sender,
            signal,
            reply_timeout,
            file_state_timeout,
        }
    }

    fn request(
        &self,
        operation: BrokerOperation,
        timeout: Duration,
    ) -> Result<BrokerResponse, PluginRuntimeError> {
        let (reply_sender, reply_receiver) = sync_channel(1);
        let request = BrokerRequest {
            scope: self.scope.clone(),
            operation,
            expires_at: Instant::now() + timeout,
            reply: reply_sender,
        };
        match self.request_sender.try_send(request) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                return Err(PluginRuntimeError::new(
                    "post_trace_host_full",
                    "daemon host request queue is full",
                ));
            }
            Err(TrySendError::Disconnected(_)) => {
                return Err(PluginRuntimeError::new(
                    "post_trace_host_closed",
                    "daemon host request broker is closed",
                ));
            }
        }
        let _ = self.signal.notify();
        match reply_receiver.recv_timeout(timeout) {
            Ok(response) => response,
            Err(RecvTimeoutError::Timeout) => Err(PluginRuntimeError::new(
                "post_trace_host_timeout",
                "daemon host request timed out",
            )),
            Err(RecvTimeoutError::Disconnected) => Err(PluginRuntimeError::new(
                "post_trace_host_closed",
                "daemon host request reply channel closed",
            )),
        }
    }
}

impl PostTraceHost for PostTraceHostClient {
    fn analysis_context(
        &self,
        trace_id: TraceId,
    ) -> Result<TraceAnalysisContext, PluginRuntimeError> {
        match self.request(
            BrokerOperation::AnalysisContext { trace_id },
            self.reply_timeout,
        )? {
            BrokerResponse::AnalysisContext(context) => Ok(context),
            _ => Err(invalid_broker_response()),
        }
    }

    fn semantic_actions_page(
        &self,
        trace_id: TraceId,
        offset: usize,
        limit: usize,
    ) -> Result<TraceAnalysisActionPage, PluginRuntimeError> {
        match self.request(
            BrokerOperation::SemanticActionsPage {
                trace_id,
                offset,
                limit,
            },
            self.reply_timeout,
        )? {
            BrokerResponse::SemanticActionsPage(page) => Ok(page),
            _ => Err(invalid_broker_response()),
        }
    }

    fn file_state(
        &self,
        trace_id: TraceId,
        action_id: &str,
    ) -> Result<TraceFileState, PluginRuntimeError> {
        match self.request(
            BrokerOperation::FileState {
                trace_id,
                action_id: action_id.to_string(),
            },
            self.file_state_timeout,
        )? {
            BrokerResponse::FileState(state) => Ok(state),
            _ => Err(invalid_broker_response()),
        }
    }
}

#[derive(Clone)]
pub(super) struct PluginScope {
    pub(super) instance_id: String,
    pub(super) plugin_id: String,
}

pub(super) struct BrokerRequest {
    pub(super) scope: PluginScope,
    pub(super) operation: BrokerOperation,
    pub(super) expires_at: Instant,
    pub(super) reply: SyncSender<Result<BrokerResponse, PluginRuntimeError>>,
}

pub(super) enum BrokerOperation {
    AnalysisContext {
        trace_id: TraceId,
    },
    SemanticActionsPage {
        trace_id: TraceId,
        offset: usize,
        limit: usize,
    },
    FileState {
        trace_id: TraceId,
        action_id: String,
    },
}

pub(super) enum BrokerResponse {
    AnalysisContext(TraceAnalysisContext),
    SemanticActionsPage(TraceAnalysisActionPage),
    FileState(TraceFileState),
}

pub(super) struct EventSignal {
    fd: OwnedFd,
}

impl EventSignal {
    pub(super) fn new() -> Result<Self, ControlError> {
        let fd = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
        if fd < 0 {
            return Err(ControlError::new(
                "post_trace_broker",
                format!("create eventfd failed: {}", std::io::Error::last_os_error()),
            ));
        }
        Ok(Self {
            fd: unsafe { OwnedFd::from_raw_fd(fd) },
        })
    }

    pub(super) fn as_raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }

    pub(super) fn notify(&self) -> std::io::Result<()> {
        let value = 1_u64.to_ne_bytes();
        let written = unsafe {
            libc::write(
                self.as_raw_fd(),
                value.as_ptr().cast::<libc::c_void>(),
                value.len(),
            )
        };
        if written == value.len() as isize
            || (written < 0
                && std::io::Error::last_os_error().kind() == std::io::ErrorKind::WouldBlock)
        {
            return Ok(());
        }
        Err(std::io::Error::last_os_error())
    }

    pub(super) fn drain(&self) -> Result<(), ControlError> {
        loop {
            let mut value = 0_u64;
            let read = unsafe {
                libc::read(
                    self.as_raw_fd(),
                    (&mut value as *mut u64).cast::<libc::c_void>(),
                    std::mem::size_of::<u64>(),
                )
            };
            if read == std::mem::size_of::<u64>() as isize {
                continue;
            }
            if read < 0 {
                let error = std::io::Error::last_os_error();
                if error.kind() == std::io::ErrorKind::WouldBlock {
                    return Ok(());
                }
                if error.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(ControlError::new(
                    "post_trace_broker",
                    format!("read eventfd failed: {error}"),
                ));
            }
            return Err(ControlError::new(
                "post_trace_broker",
                format!("eventfd returned short read {read}"),
            ));
        }
    }
}

fn invalid_broker_response() -> PluginRuntimeError {
    PluginRuntimeError::new(
        "post_trace_host_protocol",
        "daemon host broker returned an unexpected response",
    )
}
