use std::collections::BTreeMap;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::mpsc::{SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use alert_contract::AlertDraft;
use control_contract::reply::ControlError;
use model_core::ids::TraceId;
use model_core::trace::TraceAlertToken;
use plugin_system::{AlertHost, PluginRuntimeError};

use super::system::FileAccessBoundaryAlert;

pub(super) struct AlertHostClient {
    admission: Arc<AlertAdmission>,
    request_sender: SyncSender<AlertRequest>,
    signal: Arc<EventSignal>,
}

impl AlertHostClient {
    pub(super) fn new(
        admission: Arc<AlertAdmission>,
        request_sender: SyncSender<AlertRequest>,
        signal: Arc<EventSignal>,
    ) -> Self {
        Self {
            admission,
            request_sender,
            signal,
        }
    }

    pub(super) fn submit_file_access_boundary_alert(
        &self,
        trace_id: TraceId,
        alert_token: TraceAlertToken,
        alert: FileAccessBoundaryAlert,
    ) -> Result<(), PluginRuntimeError> {
        self.enqueue(
            trace_id,
            alert_token,
            QueuedAlertDraft::FileAccessBoundary(alert),
        )
    }

    fn enqueue(
        &self,
        trace_id: TraceId,
        alert_token: TraceAlertToken,
        draft: QueuedAlertDraft,
    ) -> Result<(), PluginRuntimeError> {
        self.admission.begin()?;
        let request = AlertRequest {
            admission: Arc::clone(&self.admission),
            trace_id,
            alert_token,
            draft,
            created_at: SystemTime::now(),
        };
        match self.request_sender.try_send(request) {
            Ok(()) => {}
            Err(TrySendError::Full(request)) => {
                request.admission.complete_runtime()?;
                return Err(PluginRuntimeError::new(
                    "alert_ingress_full",
                    "daemon alert ingress queue is full",
                ));
            }
            Err(TrySendError::Disconnected(request)) => {
                request.admission.complete_runtime()?;
                return Err(PluginRuntimeError::new(
                    "alert_ingress_closed",
                    "daemon alert ingress is closed",
                ));
            }
        }
        if let Err(error) = self.signal.notify() {
            tracing::error!(
                error = %error,
                "accepted alert could not signal the daemon event loop"
            );
        }
        Ok(())
    }
}

impl AlertHost for AlertHostClient {
    fn submit_alert(
        &self,
        trace_id: TraceId,
        alert_token: TraceAlertToken,
        draft: AlertDraft,
    ) -> Result<(), PluginRuntimeError> {
        self.admission.validate(&draft)?;
        self.enqueue(trace_id, alert_token, QueuedAlertDraft::Ready(draft))
    }
}

pub(super) struct AlertAdmission {
    pub(super) instance_id: String,
    pub(super) plugin_id: String,
    outputs: BTreeMap<String, RegisteredOutput>,
    state: Mutex<AdmissionState>,
}

impl AlertAdmission {
    pub(super) fn new(
        instance_id: String,
        plugin_id: String,
        outputs: BTreeMap<String, RegisteredOutput>,
    ) -> Self {
        Self {
            instance_id,
            plugin_id,
            outputs,
            state: Mutex::new(AdmissionState {
                accepting: true,
                outstanding_writes: 0,
            }),
        }
    }

    pub(super) fn validate(&self, draft: &AlertDraft) -> Result<(), PluginRuntimeError> {
        let output = self.outputs.get(&draft.definition_key).ok_or_else(|| {
            PluginRuntimeError::new(
                "alert_definition",
                format!(
                    "plugin {} did not declare alert definition {}",
                    self.plugin_id, draft.definition_key
                ),
            )
        })?;
        output.validate_payload(&draft.payload_json)
    }

    pub(super) fn close(&self) -> Result<(), ControlError> {
        self.state
            .lock()
            .map_err(admission_control_error)?
            .accepting = false;
        Ok(())
    }

    pub(super) fn has_outstanding_writes(&self) -> Result<bool, ControlError> {
        Ok(self
            .state
            .lock()
            .map_err(admission_control_error)?
            .outstanding_writes
            > 0)
    }

    pub(super) fn complete(&self) -> Result<(), ControlError> {
        let mut state = self.state.lock().map_err(admission_control_error)?;
        state.complete().map_err(|message| {
            ControlError::new(
                "alert_ingress_accounting",
                format!("plugin {}: {message}", self.instance_id),
            )
        })
    }

    fn begin(&self) -> Result<(), PluginRuntimeError> {
        let mut state = self.state.lock().map_err(admission_runtime_error)?;
        if !state.accepting {
            return Err(PluginRuntimeError::new(
                "alert_ingress_closed",
                "plugin alert admission is closed",
            ));
        }
        state.outstanding_writes = state.outstanding_writes.checked_add(1).ok_or_else(|| {
            PluginRuntimeError::new("alert_ingress_accounting", "alert write count overflow")
        })?;
        Ok(())
    }

    fn complete_runtime(&self) -> Result<(), PluginRuntimeError> {
        let mut state = self.state.lock().map_err(admission_runtime_error)?;
        state
            .complete()
            .map_err(|message| PluginRuntimeError::new("alert_ingress_accounting", message))
    }
}

struct AdmissionState {
    accepting: bool,
    outstanding_writes: usize,
}

impl AdmissionState {
    fn complete(&mut self) -> Result<(), String> {
        self.outstanding_writes = self
            .outstanding_writes
            .checked_sub(1)
            .ok_or_else(|| "alert write count underflow".to_string())?;
        Ok(())
    }
}

pub(super) struct RegisteredOutput {
    validator: jsonschema::Validator,
}

impl RegisteredOutput {
    pub(super) fn new(validator: jsonschema::Validator) -> Self {
        Self { validator }
    }

    fn validate_payload(&self, payload_json: &str) -> Result<(), PluginRuntimeError> {
        let payload = serde_json::from_str::<serde_json::Value>(payload_json).map_err(|error| {
            PluginRuntimeError::new(
                "alert_payload",
                format!("alert payload is not valid JSON: {error}"),
            )
        })?;
        let errors = self
            .validator
            .iter_errors(&payload)
            .take(3)
            .collect::<Vec<_>>();
        if errors.is_empty() {
            return Ok(());
        }
        Err(PluginRuntimeError::new(
            "alert_payload",
            errors
                .iter()
                .map(|error| format!("{}: {error}", error.instance_path()))
                .collect::<Vec<_>>()
                .join("; "),
        ))
    }
}

pub(super) struct AlertRequest {
    pub(super) admission: Arc<AlertAdmission>,
    pub(super) trace_id: TraceId,
    pub(super) alert_token: TraceAlertToken,
    pub(super) draft: QueuedAlertDraft,
    pub(super) created_at: SystemTime,
}

pub(super) enum QueuedAlertDraft {
    Ready(AlertDraft),
    FileAccessBoundary(FileAccessBoundaryAlert),
}

impl QueuedAlertDraft {
    pub(super) fn into_draft(self) -> Result<AlertDraft, PluginRuntimeError> {
        match self {
            Self::Ready(draft) => Ok(draft),
            Self::FileAccessBoundary(alert) => alert.into_draft(),
        }
    }
}

pub(super) struct EventSignal {
    fd: OwnedFd,
}

impl EventSignal {
    pub(super) fn new() -> Result<Self, ControlError> {
        let fd = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
        if fd < 0 {
            return Err(ControlError::new(
                "alert_ingress",
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
                    "alert_ingress",
                    format!("read eventfd failed: {error}"),
                ));
            }
            return Err(ControlError::new(
                "alert_ingress",
                format!("eventfd returned short read {read}"),
            ));
        }
    }
}

fn admission_runtime_error<T>(error: std::sync::PoisonError<T>) -> PluginRuntimeError {
    PluginRuntimeError::new(
        "alert_ingress_accounting",
        format!("alert admission lock poisoned: {error}"),
    )
}

fn admission_control_error<T>(error: std::sync::PoisonError<T>) -> ControlError {
    ControlError::new(
        "alert_ingress_accounting",
        format!("alert admission lock poisoned: {error}"),
    )
}
